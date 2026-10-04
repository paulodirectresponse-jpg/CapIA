//! Artefatos **derivados** de um asset (índice de quadros, waveform, proxy): a lógica de produzir
//! cada um com cache atômico, sem conhecer jobs. O pipeline de jobs e as chamadas síncronas usam
//! estas mesmas funções. Tudo aqui é regenerável e descartável (ADR-049/057).

use capia_assets::{
    AssetError, AssetErrorCode, AssetRecord, CacheDir, CacheKey, Produced, file_stamp,
    fingerprint_file,
};
use capia_media::{
    AUDIO_INDEX_PRODUCER, AudioIndex, AudioPcm, DecodeLimits, FrameIndex, INDEX_PRODUCER,
    IndexOptions, MediaToolchain, PROXY_PRODUCER, ProxyProfileV1, RawFrame, WAVEFORM_PRODUCER,
    Waveform, build_audio_index, build_frame_index, container_supports_exact_seek, decode_frame_at,
    decode_frame_by_index, generate_proxy, generate_waveform,
};
use capia_time::Ticks;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Duration;

pub(crate) type Cancel<'a> = &'a dyn Fn() -> bool;

fn media_err(e: capia_media::MediaError) -> AssetError {
    e.into()
}

fn io(what: &str, e: std::io::Error) -> AssetError {
    AssetError::new(AssetErrorCode::AssetIo, format!("{what}: {e}"))
}

/// O arquivo ainda é (provavelmente) o conteúdo do asset: tamanho + impressão rápida. O SHA-256
/// completo é do `verify`; aqui evitamos gravar no cache, sob o hash do asset, o derivado de um
/// arquivo trocado (a impressão amostrada pega a troca típica; ver ADR-053).
pub(crate) fn check_source(rec: &AssetRecord, file: &Path) -> Result<(), AssetError> {
    let mismatch = |why: &str| {
        AssetError::new(
            AssetErrorCode::AssetHashMismatch,
            format!(
                "`{}` no longer matches asset {} ({why}); run `verify` before deriving media",
                file.display(),
                rec.asset_id
            ),
        )
    };
    let st = file_stamp(file)?;
    if st.size != rec.size_bytes {
        return Err(mismatch("size differs"));
    }
    if let Some(want) = &rec.fingerprint
        && fingerprint_file(file)?.to_text() != *want
    {
        return Err(mismatch("fingerprint differs"));
    }
    Ok(())
}

/// Depois de gerar: o arquivo não pode ter mudado durante o processamento.
fn unchanged_since(file: &Path, before: capia_assets::FileStamp) -> Result<(), AssetError> {
    if file_stamp(file)? == before {
        Ok(())
    } else {
        Err(AssetError::new(
            AssetErrorCode::AssetChangedDuringProcessing,
            format!("`{}` changed while it was being processed", file.display()),
        ))
    }
}

pub(crate) struct Env<'a> {
    pub toolchain: &'a MediaToolchain,
    pub cache: &'a CacheDir,
}

// ---- chaves ----------------------------------------------------------------------------------

fn index_key(env: &Env<'_>, rec: &AssetRecord, stream: u32) -> Result<CacheKey, AssetError> {
    CacheKey::new(
        &rec.content_hash,
        "frame-index",
        &[("stream", &stream.to_string())],
        &format!("{INDEX_PRODUCER};{}", env.toolchain.version),
    )
}

fn waveform_key(
    env: &Env<'_>,
    rec: &AssetRecord,
    stream: u32,
    rate: u32,
) -> Result<CacheKey, AssetError> {
    CacheKey::new(
        &rec.content_hash,
        "waveform",
        &[("stream", &stream.to_string()), ("rate", &rate.to_string())],
        &format!("{WAVEFORM_PRODUCER};{}", env.toolchain.version),
    )
}

fn audio_index_key(env: &Env<'_>, rec: &AssetRecord, stream: u32) -> Result<CacheKey, AssetError> {
    CacheKey::new(
        &rec.content_hash,
        "audio-index",
        &[("stream", &stream.to_string())],
        &format!("{AUDIO_INDEX_PRODUCER};{}", env.toolchain.version),
    )
}

fn proxy_key(
    env: &Env<'_>,
    rec: &AssetRecord,
    profile: &ProxyProfileV1,
) -> Result<CacheKey, AssetError> {
    CacheKey::new(
        &rec.content_hash,
        "proxy",
        &[("profile", &profile.cache_fragment())],
        &format!("{PROXY_PRODUCER};{}", env.toolchain.version),
    )
}

pub(crate) fn dedup_key_index(rec: &AssetRecord) -> String {
    format!("index:{}", rec.content_hash)
}

pub(crate) fn dedup_key_waveform(rec: &AssetRecord) -> String {
    format!("waveform:{}", rec.content_hash)
}

pub(crate) fn dedup_key_audio_index(rec: &AssetRecord) -> String {
    format!("audio-index:{}", rec.content_hash)
}

pub(crate) fn dedup_key_proxy(rec: &AssetRecord, profile: &ProxyProfileV1) -> String {
    format!("proxy:{}:{}", rec.content_hash, profile.cache_fragment())
}

// ---- índice de quadros -----------------------------------------------------------------------

pub(crate) fn load_frame_index(path: &Path) -> Result<FrameIndex, AssetError> {
    let bytes = std::fs::read(path).map_err(|e| io("cannot read the frame index", e))?;
    FrameIndex::decode(&bytes).map_err(media_err)
}

fn video_of(rec: &AssetRecord) -> Result<&capia_media::VideoStream, AssetError> {
    rec.media.video().ok_or_else(|| {
        AssetError::new(
            AssetErrorCode::Media(capia_media::MediaErrorCode::MediaUnsupportedFormat),
            format!("asset {} has no video stream", rec.asset_id),
        )
    })
}

pub(crate) fn ensure_frame_index(
    env: &Env<'_>,
    rec: &AssetRecord,
    file: &Path,
    cancel: Cancel<'_>,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Produced, AssetError> {
    let v = video_of(rec)?;
    let tb = v.time_base.ok_or_else(|| {
        AssetError::new(
            AssetErrorCode::Media(capia_media::MediaErrorCode::MediaMetadataInvalid),
            "the video stream has no time base",
        )
    })?;
    let key = index_key(env, rec, v.index)?;
    // hit válido não relê a fonte
    if let Some(path) = env
        .cache
        .get_valid(&key, "idx", &|p| load_frame_index(p).map(|_| ()))
    {
        return Ok(Produced { path, hit: true });
    }
    check_source(rec, file)?;
    let stamp = file_stamp(file)?;
    // estimativa p/ progresso: duração × fps médio (0 = desconhecido)
    let hint = match (rec.media.duration, v.frame_rate) {
        (Some(d), Some(fr)) if fr.den() != 0 => u64::try_from(
            i128::from(d.0.max(0)) * i128::from(fr.num())
                / i128::from(fr.den())
                / i128::from(capia_time::TICKS_PER_SECOND),
        )
        .unwrap_or(0),
        _ => 0,
    };
    let progress = std::cell::RefCell::new(progress);
    env.cache.produce(
        &key,
        "idx",
        cancel,
        &|p| load_frame_index(p).map(|_| ()),
        &|tmp| {
            let idx = build_frame_index(
                env.toolchain,
                file,
                v.index,
                tb,
                hint,
                &IndexOptions::default(),
                cancel,
                &mut |d, t| (progress.borrow_mut())(d, t),
            )
            .map_err(media_err)?;
            unchanged_since(file, stamp)?;
            std::fs::write(tmp, idx.encode()).map_err(|e| io("cannot write the frame index", e))
        },
    )
}

// ---- waveform --------------------------------------------------------------------------------

pub(crate) fn load_waveform(path: &Path) -> Result<Waveform, AssetError> {
    let bytes = std::fs::read(path).map_err(|e| io("cannot read the waveform", e))?;
    Waveform::decode(&bytes).map_err(media_err)
}

pub(crate) fn ensure_waveform(
    env: &Env<'_>,
    rec: &AssetRecord,
    file: &Path,
    cancel: Cancel<'_>,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Produced, AssetError> {
    let a = rec.media.audio().ok_or_else(|| {
        AssetError::new(
            AssetErrorCode::Media(capia_media::MediaErrorCode::MediaUnsupportedFormat),
            format!("asset {} has no audio stream", rec.asset_id),
        )
    })?;
    let key = waveform_key(env, rec, a.index, a.sample_rate)?;
    if let Some(path) = env
        .cache
        .get_valid(&key, "wfm", &|p| load_waveform(p).map(|_| ()))
    {
        return Ok(Produced { path, hit: true });
    }
    check_source(rec, file)?;
    let stamp = file_stamp(file)?;
    let total_hint = a
        .duration
        .map_or(0, |d| capia_media::ticks_to_samples(d, a.sample_rate));
    let progress = std::cell::RefCell::new(progress);
    env.cache.produce(
        &key,
        "wfm",
        cancel,
        &|p| load_waveform(p).map(|_| ()),
        &|tmp| {
            let w = generate_waveform(
                env.toolchain,
                file,
                a.index,
                a.sample_rate,
                Duration::from_secs(6 * 3600),
                cancel,
                &mut |done| (progress.borrow_mut())(done, total_hint),
            )
            .map_err(media_err)?;
            unchanged_since(file, stamp)?;
            std::fs::write(tmp, w.encode()).map_err(|e| io("cannot write the waveform", e))
        },
    )
}

// ---- índice de áudio -------------------------------------------------------------------------

pub(crate) fn load_audio_index(path: &Path) -> Result<AudioIndex, AssetError> {
    let bytes = std::fs::read(path).map_err(|e| io("cannot read the audio index", e))?;
    AudioIndex::decode(&bytes).map_err(media_err)
}

fn audio_of(rec: &AssetRecord) -> Result<&capia_media::AudioStream, AssetError> {
    rec.media.audio().ok_or_else(|| {
        AssetError::new(
            AssetErrorCode::Media(capia_media::MediaErrorCode::MediaUnsupportedFormat),
            format!("asset {} has no audio stream", rec.asset_id),
        )
    })
}

pub(crate) fn ensure_audio_index(
    env: &Env<'_>,
    rec: &AssetRecord,
    file: &Path,
    cancel: Cancel<'_>,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Produced, AssetError> {
    let a = audio_of(rec)?;
    // registros importados antes do índice de áudio não têm o *time base* do stream: relê do arquivo
    let tb = match a.time_base {
        Some(tb) => tb,
        None => {
            use capia_media::{FfprobeBackend, MediaProbe};
            let fresh = FfprobeBackend::new(env.toolchain.clone())
                .probe(file)
                .map_err(media_err)?;
            fresh
                .streams
                .iter()
                .find_map(|s| match s {
                    capia_media::StreamInfo::Audio(x) if x.index == a.index => x.time_base,
                    _ => None,
                })
                .ok_or_else(|| {
                    AssetError::new(
                        AssetErrorCode::Media(capia_media::MediaErrorCode::MediaMetadataInvalid),
                        "the audio stream has no time base",
                    )
                })?
        }
    };
    let key = audio_index_key(env, rec, a.index)?;
    if let Some(path) = env
        .cache
        .get_valid(&key, "aix", &|p| load_audio_index(p).map(|_| ()))
    {
        return Ok(Produced { path, hit: true });
    }
    check_source(rec, file)?;
    let stamp = file_stamp(file)?;
    let fast = container_supports_exact_seek(&rec.media.container.formats);
    let total_hint = a
        .duration
        .map_or(0, |d| capia_media::ticks_to_samples(d, a.sample_rate));
    let progress = std::cell::RefCell::new(progress);
    env.cache.produce(
        &key,
        "aix",
        cancel,
        &|p| load_audio_index(p).map(|_| ()),
        &|tmp| {
            let ix = build_audio_index(
                env.toolchain,
                file,
                a.index,
                a.sample_rate,
                a.channels,
                tb,
                fast,
                Duration::from_secs(6 * 3600),
                cancel,
                &mut |done| (progress.borrow_mut())(done, total_hint),
            )
            .map_err(media_err)?;
            unchanged_since(file, stamp)?;
            std::fs::write(tmp, ix.encode()).map_err(|e| io("cannot write the audio index", e))
        },
    )
}

// ---- proxy -----------------------------------------------------------------------------------

fn validate_proxy(env: &Env<'_>, path: &Path, profile: &ProxyProfileV1) -> Result<(), AssetError> {
    use capia_media::{FfprobeBackend, MediaProbe};
    let info = FfprobeBackend::new(env.toolchain.clone())
        .probe(path)
        .map_err(media_err)?;
    match info.video() {
        Some(v) if v.width <= profile.max_width && v.height <= profile.max_height => Ok(()),
        _ => Err(AssetError::new(
            AssetErrorCode::Media(capia_media::MediaErrorCode::MediaEncodeFailed),
            "the proxy file is not a valid proxy",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn ensure_proxy(
    env: &Env<'_>,
    rec: &AssetRecord,
    file: &Path,
    profile: &ProxyProfileV1,
    pid_sink: Option<Arc<AtomicU32>>,
    cancel: Cancel<'_>,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Produced, AssetError> {
    video_of(rec)?;
    profile.validate().map_err(media_err)?;
    let key = proxy_key(env, rec, profile)?;
    if let Some(path) = env
        .cache
        .get_valid(&key, "mov", &|p| validate_proxy(env, p, profile))
    {
        return Ok(Produced { path, hit: true });
    }
    check_source(rec, file)?;
    let stamp = file_stamp(file)?;
    let dur_us = rec.media.duration.map_or(0, |d| {
        u64::try_from(i128::from(d.0.max(0)) * 1_000_000 / i128::from(capia_time::TICKS_PER_SECOND))
            .unwrap_or(0)
    });
    let progress = std::cell::RefCell::new(progress);
    env.cache.produce(
        &key,
        "mov",
        cancel,
        &|p| validate_proxy(env, p, profile),
        &|tmp| {
            generate_proxy(
                env.toolchain,
                file,
                tmp,
                profile,
                rec.media.has_audio(),
                dur_us,
                Duration::from_secs(6 * 3600),
                pid_sink.clone(),
                cancel,
                &mut |d, t| (progress.borrow_mut())(d, t),
            )
            .map_err(media_err)?;
            unchanged_since(file, stamp)
        },
    )
}

// ---- decode ----------------------------------------------------------------------------------

/// Fonte de quadros precisos de um asset: o índice (em memória) + o arquivo + o ffmpeg.
#[derive(Clone, Debug)]
pub struct FrameSource {
    toolchain: MediaToolchain,
    file: PathBuf,
    index: Arc<FrameIndex>,
    width: u32,
    height: u32,
}

impl FrameSource {
    pub(crate) fn new(
        toolchain: MediaToolchain,
        file: PathBuf,
        index: FrameIndex,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            toolchain,
            file,
            index: Arc::new(index),
            width,
            height,
        }
    }

    pub fn index(&self) -> &FrameIndex {
        &self.index
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Quadro mostrado no tempo `at` (relativo ao primeiro quadro): o último com tempo ≤ `at`.
    pub fn frame_at(&self, at: Ticks, cancel: Cancel<'_>) -> Result<RawFrame, AssetError> {
        decode_frame_at(
            &self.toolchain,
            &self.file,
            &self.index,
            self.width,
            self.height,
            at,
            &DecodeLimits::default(),
            cancel,
        )
        .map_err(media_err)
    }

    /// `count` quadros consecutivos a partir do lógico `first`, com UM processo (scrub/reprodução).
    pub fn frames(
        &self,
        first: usize,
        count: usize,
        cancel: Cancel<'_>,
        on_frame: &mut dyn FnMut(RawFrame) -> capia_media::Flow,
    ) -> Result<usize, AssetError> {
        capia_media::decode_frame_range(
            &self.toolchain,
            &self.file,
            &self.index,
            self.width,
            self.height,
            first,
            count,
            &DecodeLimits::default(),
            cancel,
            on_frame,
        )
        .map_err(media_err)
    }

    pub fn frame_by_index(&self, i: usize, cancel: Cancel<'_>) -> Result<RawFrame, AssetError> {
        decode_frame_by_index(
            &self.toolchain,
            &self.file,
            &self.index,
            self.width,
            self.height,
            i,
            &DecodeLimits::default(),
            cancel,
        )
        .map_err(media_err)
    }
}

pub(crate) fn frame_source(
    env: &Env<'_>,
    rec: &AssetRecord,
    file: &Path,
    cancel: Cancel<'_>,
) -> Result<FrameSource, AssetError> {
    let v = video_of(rec)?;
    let produced = ensure_frame_index(env, rec, file, cancel, &mut |_, _| {})?;
    let index = load_frame_index(&produced.path)?;
    Ok(FrameSource::new(
        env.toolchain.clone(),
        file.to_path_buf(),
        index,
        v.width,
        v.height,
    ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_pcm(
    env: &Env<'_>,
    rec: &AssetRecord,
    file: &Path,
    start: Ticks,
    duration: Ticks,
    sample_rate: Option<u32>,
    channels: Option<u32>,
    cancel: Cancel<'_>,
) -> Result<AudioPcm, AssetError> {
    let a = rec.media.audio().ok_or_else(|| {
        AssetError::new(
            AssetErrorCode::Media(capia_media::MediaErrorCode::MediaUnsupportedFormat),
            format!("asset {} has no audio stream", rec.asset_id),
        )
    })?;
    check_source(rec, file)?;
    capia_media::decode_audio(
        env.toolchain,
        file,
        &capia_media::AudioRequest {
            stream_index: a.index,
            sample_rate: sample_rate.unwrap_or(a.sample_rate),
            channels: channels.unwrap_or(a.channels),
            start,
            duration,
        },
        capia_media::DEFAULT_MAX_PCM_BYTES,
        Duration::from_secs(600),
        cancel,
    )
    .map_err(media_err)
}
