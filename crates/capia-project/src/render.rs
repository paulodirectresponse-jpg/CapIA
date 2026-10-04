//! Render do projeto: liga o `RenderGraph`/compositor puros (`capia-render`) à mídia real —
//! decode service persistente (`capia-decode`), índice de áudio e cache de PCM. A fonte é SEMPRE o
//! original (nunca o proxy): o proxy não define a verdade do export (ADR-064).

use crate::derive::{self, Env};
use crate::error::ProjectError;
use crate::project::Project;
use capia_assets::{AssetRecord, CacheDir};
use capia_decode::{
    AudioSource, DecodeConfig, DecodeService, Lane, PcmCache, Priority as DecodePriority,
    VideoSource,
};
use capia_media::{DecodeLimits, MediaToolchain};
use capia_model::{AssetId, SequenceId};
use capia_render::{
    AudioBuffer, AudioRequest, GraphClipKind, Image, MediaSource, RenderGraph, RenderSettings,
    RenderWarning, RenderedFrame, SourceError, mix_audio_range, render_frame, render_video_range,
    resample_sinc,
};
use capia_time::{Ticks, TimeRange};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Margem (amostras de fonte) lida além do trecho para o filtro de reamostragem.
const RESAMPLE_MARGIN: u64 = 32;

/// Serviços de longa duração do render (decode persistente + cache de PCM). Um por processo/editor;
/// vários projetos podem compartilhá-lo (as chaves de cache têm namespace por projeto).
#[derive(Debug)]
pub struct RenderServices {
    decode: DecodeService,
    pcm: PcmCache,
    toolchain: MediaToolchain,
}

impl RenderServices {
    pub fn new(toolchain: MediaToolchain) -> Self {
        Self::with_config(DecodeConfig::new(toolchain), 128 * 1024 * 1024)
    }

    pub fn with_config(cfg: DecodeConfig, pcm_cache_bytes: u64) -> Self {
        let toolchain = cfg.toolchain.clone();
        Self {
            pcm: PcmCache::new(toolchain.clone(), pcm_cache_bytes),
            decode: DecodeService::new(cfg),
            toolchain,
        }
    }

    pub fn decode(&self) -> &DecodeService {
        &self.decode
    }

    pub fn pcm(&self) -> &PcmCache {
        &self.pcm
    }

    pub fn toolchain(&self) -> &MediaToolchain {
        &self.toolchain
    }
}

/// Como a fonte pede quadros ao decode service.
#[derive(Clone, Copy, Debug)]
pub struct SourceOptions {
    pub priority: DecodePriority,
    pub lane: Option<Lane>,
}

impl Default for SourceOptions {
    fn default() -> Self {
        Self {
            priority: DecodePriority::Playback,
            lane: None,
        }
    }
}

#[derive(Debug)]
struct Entry {
    rec: AssetRecord,
    file: Option<PathBuf>,
}

/// `MediaSource` do projeto: resolve assets do catálogo, gera/lê os derivados do cache e decodifica.
#[derive(Debug)]
pub struct ProjectSource {
    services: Arc<RenderServices>,
    namespace: u64,
    cache: CacheDir,
    entries: BTreeMap<AssetId, Entry>,
    opts: SourceOptions,
    inflight: Mutex<Vec<capia_decode::FrameTicket>>,
    video: Mutex<HashMap<AssetId, Arc<VideoSource>>>,
    audio: Mutex<HashMap<AssetId, AudioSource>>,
    stills: Mutex<HashMap<AssetId, Arc<Image>>>,
}

/// Um quadro pronto para renderizar, desacoplado do `Project` (ver [`Project::prepare_frame`]).
#[derive(Debug)]
pub struct FrameJob {
    graph: Arc<RenderGraph>,
    source: ProjectSource,
    seq: SequenceId,
}

impl FrameJob {
    pub fn render(
        &self,
        time: Ticks,
        settings: &RenderSettings,
    ) -> Result<RenderedFrame, ProjectError> {
        render_frame(&self.graph, &self.seq, time, settings, &self.source).map_err(render_err)
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn serr(code: &str, e: impl core::fmt::Display) -> SourceError {
    SourceError::new(code, e.to_string())
}

impl ProjectSource {
    fn entry(&self, id: &AssetId) -> Result<&Entry, SourceError> {
        self.entries.get(id).ok_or_else(|| {
            SourceError::new("ASSET_NOT_MANAGED", format!("asset {id} has no media file"))
        })
    }

    fn file<'a>(&self, e: &'a Entry) -> Result<&'a PathBuf, SourceError> {
        e.file.as_ref().ok_or_else(|| {
            SourceError::new(
                "ASSET_OFFLINE",
                format!("asset {} is offline: no file to read", e.rec.asset_id),
            )
        })
    }

    fn env(&self) -> Env<'_> {
        Env {
            toolchain: self.services.toolchain(),
            cache: &self.cache,
        }
    }

    fn video_source(&self, id: &AssetId) -> Result<Arc<VideoSource>, SourceError> {
        if let Some(v) = lock(&self.video).get(id) {
            return Ok(Arc::clone(v));
        }
        let e = self.entry(id)?;
        let file = self.file(e)?;
        let v = e.rec.media.video().ok_or_else(|| {
            SourceError::new("MEDIA_NO_VIDEO", format!("asset {id} has no video stream"))
        })?;
        let produced =
            derive::ensure_frame_index(&self.env(), &e.rec, file, &|| false, &mut |_, _| {})
                .map_err(|er| serr(&er.code.to_string(), er.message))?;
        let index = derive::load_frame_index(&produced.path)
            .map_err(|er| serr(&er.code.to_string(), er.message))?;
        let src = Arc::new(VideoSource {
            namespace: self.namespace,
            content: Arc::from(e.rec.content_hash.as_str()),
            path: file.clone(),
            index: Arc::new(index),
            width: v.width,
            height: v.height,
        });
        lock(&self.video).insert(id.clone(), Arc::clone(&src));
        Ok(src)
    }

    fn audio_source(&self, id: &AssetId) -> Result<AudioSource, SourceError> {
        if let Some(a) = lock(&self.audio).get(id) {
            return Ok(a.clone());
        }
        let e = self.entry(id)?;
        let file = self.file(e)?;
        let produced =
            derive::ensure_audio_index(&self.env(), &e.rec, file, &|| false, &mut |_, _| {})
                .map_err(|er| serr(&er.code.to_string(), er.message))?;
        let index = derive::load_audio_index(&produced.path)
            .map_err(|er| serr(&er.code.to_string(), er.message))?;
        let src = AudioSource {
            namespace: self.namespace,
            content: Arc::from(e.rec.content_hash.as_str()),
            path: file.clone(),
            index: Arc::new(index),
        };
        lock(&self.audio).insert(id.clone(), src.clone());
        Ok(src)
    }
}

/// Converte entre números de canais: igual ⇒ cópia; mono → N duplica; N → mono média; senão copia
/// os canais em comum e zera o resto.
fn remap_channels(src: &[f32], from: usize, to: usize) -> Vec<f32> {
    if from == to || from == 0 {
        return src.to_vec();
    }
    let frames = src.len() / from;
    let mut out = vec![0.0f32; frames * to];
    for f in 0..frames {
        let s = &src[f * from..(f + 1) * from];
        let d = &mut out[f * to..(f + 1) * to];
        if from == 1 {
            d.fill(s[0]);
        } else if to == 1 {
            d[0] = s.iter().sum::<f32>() / from as f32;
        } else {
            let n = from.min(to);
            d[..n].copy_from_slice(&s[..n]);
        }
    }
    out
}

impl MediaSource for ProjectSource {
    fn video_frame(
        &self,
        asset: &AssetId,
        source_t: Ticks,
    ) -> Result<Option<Arc<Image>>, SourceError> {
        let v = self.video_source(asset)?;
        let Some(i) = v.index.frame_at_or_before(source_t) else {
            return Ok(None);
        };
        let ticket = self
            .services
            .decode()
            .request(&v, i, self.opts.priority, self.opts.lane);
        {
            let mut g = lock(&self.inflight);
            g.retain(|t| !t.is_done());
            g.push(ticket.clone());
        }
        let f = ticket.wait().map_err(|e| match e {
            capia_decode::DecodeError::Cancelled | capia_decode::DecodeError::Superseded => {
                serr("DECODE_CANCELLED", e)
            }
            other => serr("MEDIA_DECODE_FAILED", other),
        })?;
        let img = Image::from_rgba(f.width, f.height, f.bytes.clone())
            .map_err(|e| serr(e.code, e.message))?;
        Ok(Some(Arc::new(img)))
    }

    fn still_image(&self, asset: &AssetId) -> Result<Arc<Image>, SourceError> {
        if let Some(i) = lock(&self.stills).get(asset) {
            return Ok(Arc::clone(i));
        }
        let e = self.entry(asset)?;
        let file = self.file(e)?;
        let v = e.rec.media.video().ok_or_else(|| {
            SourceError::new(
                "MEDIA_NO_VIDEO",
                format!("asset {asset} has no image stream"),
            )
        })?;
        derive::check_source(&e.rec, file).map_err(|er| serr(&er.code.to_string(), er.message))?;
        let f = capia_media::decode_still(
            self.services.toolchain(),
            file,
            v.width,
            v.height,
            &DecodeLimits::default(),
            &|| false,
        )
        .map_err(|er| serr(er.code.as_str(), er.message))?;
        let img = Arc::new(
            Image::from_rgba(f.width, f.height, f.bytes).map_err(|er| serr(er.code, er.message))?,
        );
        lock(&self.stills).insert(asset.clone(), Arc::clone(&img));
        Ok(img)
    }

    fn cancel_pending(&self) {
        for t in lock(&self.inflight).drain(..) {
            t.cancel();
        }
    }

    fn audio(&self, asset: &AssetId, req: AudioRequest) -> Result<AudioBuffer, SourceError> {
        let src = self.audio_source(asset)?;
        let (n_rate, n_ch) = (src.index.sample_rate(), src.index.channels() as usize);
        let (o_rate, o_ch) = (req.sample_rate, req.channels as usize);
        let rate_n = u128::from(n_rate);
        let rate_o = u128::from(o_rate);
        // trecho de fonte que cobre [start, start+frames) + margem do filtro
        let lo = (u128::from(req.start_sample) * rate_n / rate_o) as u64;
        let hi = ((u128::from(req.start_sample + req.frames) * rate_n).div_ceil(rate_o)) as u64;
        let (from, to) = if n_rate == o_rate {
            (req.start_sample, req.start_sample + req.frames)
        } else {
            (lo.saturating_sub(RESAMPLE_MARGIN), hi + RESAMPLE_MARGIN)
        };
        let pcm = self
            .services
            .pcm()
            .read(&src, from, to - from, &|| false)
            .map_err(|e| serr(e.code.as_str(), e.message))?;
        let native = resample_sinc(
            &pcm.samples,
            n_ch as u32,
            n_rate,
            o_rate,
            from,
            req.start_sample,
            req.frames,
        );
        Ok(AudioBuffer {
            sample_rate: o_rate,
            channels: req.channels,
            samples: remap_channels(&native, n_ch, o_ch),
        })
    }
}

fn render_err(e: capia_render::RenderError) -> ProjectError {
    ProjectError::Invalid {
        code: e.code,
        message: e.message,
        details: None,
    }
}

fn fnv1a(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl Project {
    /// Compila o grafo de render de `seq` (e das sequences aninhadas) a partir do documento atual.
    pub fn render_graph(&self, seq: &SequenceId) -> Result<RenderGraph, ProjectError> {
        RenderGraph::compile(self.document(), seq).map_err(render_err)
    }

    /// Fonte de mídia para o render do projeto. Resolve os assets que o grafo referencia;
    /// assets offline ficam registrados como indisponíveis (erro só se forem usados).
    pub fn render_source(
        &self,
        services: &Arc<RenderServices>,
        graph: &RenderGraph,
        opts: SourceOptions,
    ) -> Result<ProjectSource, ProjectError> {
        let mut wanted: Vec<AssetId> = Vec::new();
        for s in graph.sequences.values() {
            for t in &s.tracks {
                for c in &t.clips {
                    if let GraphClipKind::Media { asset, .. } | GraphClipKind::Image { asset } =
                        &c.kind
                        && !wanted.contains(asset)
                    {
                        wanted.push(asset.clone());
                    }
                }
            }
        }
        let dir = self.project_dir();
        let mut entries = BTreeMap::new();
        for id in wanted {
            let Ok(rec) = self.managed(&id) else { continue };
            let (status, found) = capia_assets::quick_status(&rec, dir.as_deref());
            let file = found.filter(|_| status == capia_assets::Availability::Online);
            entries.insert(id, Entry { rec, file });
        }
        let abs = std::path::absolute(self.path()).unwrap_or_else(|_| self.path().to_path_buf());
        Ok(ProjectSource {
            services: Arc::clone(services),
            namespace: fnv1a(&abs.to_string_lossy()),
            cache: self.cache_dir(),
            entries,
            opts,
            inflight: Mutex::new(Vec::new()),
            video: Mutex::new(HashMap::new()),
            audio: Mutex::new(HashMap::new()),
            stills: Mutex::new(HashMap::new()),
        })
    }

    /// Grafo de `seq` reaproveitado enquanto a revisão do documento é a mesma (a revisão muda em
    /// todo commit/undo/redo, então o cache nunca serve um grafo antigo).
    pub fn render_graph_cached(&self, seq: &SequenceId) -> Result<Arc<RenderGraph>, ProjectError> {
        let revision = self.document().revision;
        if let Ok(guard) = self.graph_cache.lock()
            && let Some((rev, id, graph)) = guard.as_ref()
            && *rev == revision
            && id == seq
        {
            return Ok(Arc::clone(graph));
        }
        let graph = Arc::new(self.render_graph(seq)?);
        if let Ok(mut guard) = self.graph_cache.lock() {
            *guard = Some((revision, seq.clone(), Arc::clone(&graph)));
        }
        Ok(graph)
    }

    /// Prepara um quadro **sem renderizar**: resolve grafo e fonte (rápido, precisa de `&self`).
    /// O [`FrameJob`] é independente do projeto e pode ser executado fora do lock da sessão, de
    /// modo que um comando/undo nunca espera o compositor nem o decode.
    pub fn prepare_frame(
        &self,
        services: &Arc<RenderServices>,
        seq: &SequenceId,
    ) -> Result<FrameJob, ProjectError> {
        let graph = self.render_graph_cached(seq)?;
        let source = self.render_source(
            services,
            &graph,
            SourceOptions {
                priority: DecodePriority::Interactive,
                lane: None,
            },
        )?;
        Ok(FrameJob {
            graph,
            source,
            seq: seq.clone(),
        })
    }

    /// Renderiza o quadro de `seq` em `time` (preto opaco por baixo; ver ADR-064).
    pub fn render_frame(
        &self,
        services: &Arc<RenderServices>,
        seq: &SequenceId,
        time: Ticks,
        settings: &RenderSettings,
    ) -> Result<RenderedFrame, ProjectError> {
        self.prepare_frame(services, seq)?.render(time, settings)
    }

    /// Renderiza os quadros de `range` na cadência de `settings` (ou da sequence).
    pub fn render_range(
        &self,
        services: &Arc<RenderServices>,
        seq: &SequenceId,
        range: TimeRange,
        settings: &RenderSettings,
        on_frame: &mut dyn FnMut(i64, Ticks, Image) -> bool,
    ) -> Result<(u64, Vec<RenderWarning>), ProjectError> {
        let graph = self.render_graph(seq)?;
        let source = self.render_source(services, &graph, SourceOptions::default())?;
        render_video_range(&graph, seq, range, settings, &source, on_frame).map_err(render_err)
    }

    /// Mix de áudio de `range` na taxa/canais de `settings`.
    pub fn render_audio_range(
        &self,
        services: &Arc<RenderServices>,
        seq: &SequenceId,
        range: TimeRange,
        settings: &RenderSettings,
    ) -> Result<(AudioBuffer, Vec<RenderWarning>), ProjectError> {
        let graph = self.render_graph(seq)?;
        let source = self.render_source(services, &graph, SourceOptions::default())?;
        mix_audio_range(&graph, seq, range, settings, &source).map_err(render_err)
    }
}
