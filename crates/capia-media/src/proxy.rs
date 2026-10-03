//! Proxy de edição (ADR-056): cópia **leve e rápida de decodificar** da mídia original, só para
//! preview/scrub. **Nunca** é fonte de verdade: a timeline e o export referenciam o original; o
//! proxy é cache descartável, regenerável e tem a chave do perfil na `CacheKey`.
//!
//! Build padrão = LGPL (ADR-032): o perfil padrão usa **MJPEG** (encoder nativo do FFmpeg, intra-
//! only ⇒ seek/scrub quadro a quadro baratos) em `.mov` + AAC nativo. H.264 só via encoder de
//! **hardware/SO** (nvenc, qsv, videotoolbox, mf…); `libx264`/`libx265` (GPL) nunca são usados.

use crate::error::{MediaError, MediaErrorCode};
use crate::failpoints::fp;
use crate::ffprobe::{FfprobeBackend, MediaProbe, checked_input_path, file_url_arg};
use crate::info::MediaInfo;
use crate::process::{Flow, StreamLimits, run_collect, run_streaming};
use crate::toolchain::MediaToolchain;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

/// Versão do perfil + algoritmo (entra na `CacheKey`).
pub const PROXY_PRODUCER: &str = "proxy/1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyCodec {
    /// MJPEG intra-only (sempre disponível, LGPL).
    Mjpeg,
    /// H.264 por encoder de hardware/SO, se existir; senão `MEDIA_ENCODER_UNAVAILABLE`.
    H264Hardware,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "fps")]
pub enum FpsPolicy {
    /// Preserva os timestamps originais (VFR continua VFR: os quadros do proxy ↔ os do original).
    Keep,
    /// Força CFR (apenas para perfis que aceitam perder correspondência quadro a quadro).
    Fixed(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "kbps")]
pub enum ProxyAudio {
    None,
    Aac(u32),
}

/// `ProxyProfileV1`: tudo o que muda o resultado está aqui (⇒ entra na chave de cache).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyProfileV1 {
    pub max_width: u32,
    pub max_height: u32,
    pub fps: FpsPolicy,
    pub codec: ProxyCodec,
    /// MJPEG: `-q:v` (2 = melhor … 31 = pior).
    pub jpeg_quality: u8,
    /// H.264: bitrate em kbps.
    pub h264_kbps: u32,
    /// H.264: intervalo de keyframes.
    pub gop: u32,
    pub audio: ProxyAudio,
}

impl Default for ProxyProfileV1 {
    fn default() -> Self {
        Self {
            max_width: 960,
            max_height: 540,
            fps: FpsPolicy::Keep,
            codec: ProxyCodec::Mjpeg,
            jpeg_quality: 5,
            h264_kbps: 4000,
            gop: 12,
            audio: ProxyAudio::Aac(128),
        }
    }
}

impl ProxyProfileV1 {
    pub fn validate(&self) -> Result<(), MediaError> {
        let bad = |m: &str| MediaError::new(MediaErrorCode::MediaMetadataInvalid, m.to_owned());
        if !(16..=8192).contains(&self.max_width) || !(16..=8192).contains(&self.max_height) {
            return Err(bad("proxy size must be within 16..=8192"));
        }
        if !(2..=31).contains(&self.jpeg_quality) {
            return Err(bad("jpeg quality must be within 2..=31"));
        }
        if !(100..=100_000).contains(&self.h264_kbps) || !(1..=600).contains(&self.gop) {
            return Err(bad("invalid H.264 bitrate or GOP"));
        }
        if let FpsPolicy::Fixed(n) = self.fps
            && !(1..=240).contains(&n)
        {
            return Err(bad("fixed fps must be within 1..=240"));
        }
        if let ProxyAudio::Aac(k) = self.audio
            && !(16..=512).contains(&k)
        {
            return Err(bad("aac bitrate must be within 16..=512 kbps"));
        }
        Ok(())
    }

    /// Texto canônico e estável do perfil para a `CacheKey`.
    pub fn cache_fragment(&self) -> String {
        // serde_json com campos em ordem de declaração é determinístico
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Encoders **permitidos** por plataforma, em ordem de preferência. Nunca `libx264`/`libx265`.
pub const HARDWARE_H264_ENCODERS: &[&str] = if cfg!(target_os = "macos") {
    &["h264_videotoolbox"]
} else if cfg!(windows) {
    &["h264_nvenc", "h264_qsv", "h264_amf", "h264_mf"]
} else {
    &["h264_nvenc", "h264_qsv", "h264_vaapi"]
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyEncoder {
    pub name: String,
    pub codec: ProxyCodec,
}

/// Lista os encoders do ffmpeg (`-encoders`).
pub fn list_encoders(tc: &MediaToolchain) -> Result<Vec<String>, MediaError> {
    let ffmpeg = tc.ffmpeg.as_deref().ok_or_else(|| {
        MediaError::new(MediaErrorCode::MediaBackendNotFound, "ffmpeg was not found")
    })?;
    let args: Vec<OsString> = ["-hide_banner", "-v", "error", "-encoders"]
        .iter()
        .map(OsString::from)
        .collect();
    let out = run_collect(
        ffmpeg,
        &args,
        &StreamLimits::new(Duration::from_secs(20)),
        1 << 20,
        &|| false,
    )?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let flags = it.next()?;
            let name = it.next()?;
            (flags.len() == 6 && flags.starts_with('V')).then(|| name.to_owned())
        })
        .collect())
}

/// Decide o encoder a partir da lista disponível (função pura ⇒ testável sem FFmpeg).
pub fn select_encoder(codec: ProxyCodec, available: &[String]) -> Result<ProxyEncoder, MediaError> {
    match codec {
        ProxyCodec::Mjpeg => available
            .iter()
            .any(|e| e == "mjpeg")
            .then(|| ProxyEncoder {
                name: "mjpeg".into(),
                codec,
            })
            .ok_or_else(|| {
                MediaError::new(
                    MediaErrorCode::MediaEncoderUnavailable,
                    "this FFmpeg build has no mjpeg encoder",
                )
            }),
        ProxyCodec::H264Hardware => HARDWARE_H264_ENCODERS
            .iter()
            .find(|c| available.iter().any(|a| a == **c))
            .map(|n| ProxyEncoder {
                name: (*n).to_owned(),
                codec,
            })
            .ok_or_else(|| {
                MediaError::new(
                    MediaErrorCode::MediaEncoderUnavailable,
                    "no hardware H.264 encoder is available (libx264 is GPL and never used)",
                )
            }),
    }
}

/// Apaga o parcial. No Windows o handle de um processo recém-morto pode demorar alguns ms para ser
/// liberado: tenta de novo por até ~2 s em vez de deixar o arquivo para trás.
fn remove_with_retry(path: &Path) {
    for _ in 0..40 {
        match std::fs::remove_file(path) {
            Ok(()) => return,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProxyReport {
    pub encoder: ProxyEncoder,
    pub info: MediaInfo,
}

/// Gera o proxy de `input` em `output` (arquivo **temporário** do chamador — a publicação atômica é
/// da camada de cache). `duration_us_hint` calibra o progresso. Cancelar mata o ffmpeg e apaga o
/// arquivo parcial. Valida o resultado com o probe antes de devolver.
#[allow(clippy::too_many_arguments)]
pub fn generate_proxy(
    tc: &MediaToolchain,
    input: &Path,
    output: &Path,
    profile: &ProxyProfileV1,
    has_audio: bool,
    duration_us_hint: u64,
    timeout: Duration,
    pid_sink: Option<std::sync::Arc<std::sync::atomic::AtomicU32>>,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<ProxyReport, MediaError> {
    profile.validate()?;
    let ffmpeg = tc.ffmpeg.as_deref().ok_or_else(|| {
        MediaError::new(MediaErrorCode::MediaBackendNotFound, "ffmpeg was not found")
    })?;
    let encoder = select_encoder(profile.codec, &list_encoders(tc)?)?;
    let abs_in = checked_input_path(input)?;
    let abs_out = std::path::absolute(output).map_err(|e| {
        MediaError::new(
            MediaErrorCode::MediaInvalidPath,
            format!("output path: {e}"),
        )
    })?;
    let _ = std::fs::remove_file(&abs_out);
    let mut args: Vec<OsString> = [
        "-v",
        "error",
        "-nostdin",
        "-nostats",
        "-stats_period",
        "0.1",
        "-protocol_whitelist",
        "file",
        "-i",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(file_url_arg(&abs_in));
    let mut vf = format!(
        "scale='min({w},iw)':'min({h},ih)':force_original_aspect_ratio=decrease:force_divisible_by=2,format=yuv420p",
        w = profile.max_width,
        h = profile.max_height
    );
    if let FpsPolicy::Fixed(n) = profile.fps {
        vf = format!("fps={n},{vf}");
    }
    let mut rest: Vec<String> = vec![
        "-map".into(),
        "0:v:0".into(),
        "-sn".into(),
        "-dn".into(),
        "-vf".into(),
        vf,
    ];
    if matches!(profile.fps, FpsPolicy::Keep) {
        rest.extend(["-fps_mode".into(), "passthrough".into()]);
    }
    rest.extend(["-c:v".into(), encoder.name.clone()]);
    match profile.codec {
        ProxyCodec::Mjpeg => rest.extend(["-q:v".into(), profile.jpeg_quality.to_string()]),
        ProxyCodec::H264Hardware => rest.extend([
            "-b:v".into(),
            format!("{}k", profile.h264_kbps),
            "-g".into(),
            profile.gop.to_string(),
        ]),
    }
    match (profile.audio, has_audio) {
        (ProxyAudio::Aac(k), true) => rest.extend([
            "-map".into(),
            "0:a:0".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            format!("{k}k"),
        ]),
        _ => rest.push("-an".into()),
    }
    rest.extend([
        "-f".into(),
        "mov".into(),
        "-progress".into(),
        "pipe:1".into(),
    ]);
    args.extend(rest.into_iter().map(OsString::from));
    let mut out_arg = OsString::from("file:");
    out_arg.push(abs_out.as_os_str());
    args.push(out_arg);

    let mut limits = StreamLimits::new(timeout);
    limits.pid_sink = pid_sink;
    let mut carry = String::new();
    let mut seen_progress = false;
    let result = run_streaming(ffmpeg, &args, &limits, cancel, &mut |chunk| {
        carry.push_str(&String::from_utf8_lossy(chunk));
        while let Some(nl) = carry.find('\n') {
            let line: String = carry.drain(..=nl).collect();
            if let Some(v) = line.trim().strip_prefix("out_time_us=")
                && let Ok(us) = v.parse::<i64>()
            {
                if !seen_progress {
                    seen_progress = true;
                    fp!("proxy_running");
                }
                progress(us.max(0) as u64, duration_us_hint);
            }
        }
        if carry.len() > 64 * 1024 {
            carry.clear();
        }
        Ok(Flow::Continue)
    });
    let cleanup = |e: MediaError| {
        remove_with_retry(&abs_out);
        e
    };
    let out = result.map_err(cleanup)?;
    match out.status {
        Some(s) if s.success() => {}
        _ => {
            return Err(cleanup(MediaError::new(
                MediaErrorCode::MediaEncodeFailed,
                format!(
                    "ffmpeg failed to encode the proxy: {}",
                    String::from_utf8_lossy(&out.stderr)
                        .lines()
                        .last()
                        .unwrap_or("")
                ),
            )));
        }
    }
    let info = FfprobeBackend::new(tc.clone())
        .probe(&abs_out)
        .map_err(cleanup)?;
    let ok = info.video().is_some_and(|v| {
        v.width <= profile.max_width && v.height <= profile.max_height && v.width > 0
    });
    if !ok {
        return Err(cleanup(MediaError::new(
            MediaErrorCode::MediaEncodeFailed,
            "the generated proxy failed validation",
        )));
    }
    Ok(ProxyReport { encoder, info })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn default_profile_is_valid_and_lgpl_only() {
        let p = ProxyProfileV1::default();
        p.validate().unwrap();
        assert_eq!(p.codec, ProxyCodec::Mjpeg);
        assert!(HARDWARE_H264_ENCODERS.iter().all(|e| !e.contains("x264")));
    }

    #[test]
    fn invalid_profiles_are_rejected() {
        for bad in [
            ProxyProfileV1 {
                max_width: 1,
                ..Default::default()
            },
            ProxyProfileV1 {
                jpeg_quality: 1,
                ..Default::default()
            },
            ProxyProfileV1 {
                fps: FpsPolicy::Fixed(0),
                ..Default::default()
            },
            ProxyProfileV1 {
                audio: ProxyAudio::Aac(1),
                ..Default::default()
            },
            ProxyProfileV1 {
                gop: 0,
                ..Default::default()
            },
        ] {
            assert!(bad.validate().is_err());
        }
    }

    #[test]
    fn encoder_selection_never_picks_gpl_encoders() {
        let all = names(&["libx264", "libx265", "mjpeg"]);
        assert_eq!(
            select_encoder(ProxyCodec::Mjpeg, &all).unwrap().name,
            "mjpeg"
        );
        let err = select_encoder(ProxyCodec::H264Hardware, &all).unwrap_err();
        assert_eq!(err.code, MediaErrorCode::MediaEncoderUnavailable);
        let hw = names(&["libx264", HARDWARE_H264_ENCODERS[0]]);
        assert_eq!(
            select_encoder(ProxyCodec::H264Hardware, &hw).unwrap().name,
            HARDWARE_H264_ENCODERS[0]
        );
        assert!(select_encoder(ProxyCodec::Mjpeg, &names(&["libx264"])).is_err());
    }

    #[test]
    fn cache_fragment_changes_with_every_field() {
        let base = ProxyProfileV1::default();
        let f0 = base.cache_fragment();
        let variants = [
            ProxyProfileV1 {
                max_width: 640,
                ..base.clone()
            },
            ProxyProfileV1 {
                max_height: 360,
                ..base.clone()
            },
            ProxyProfileV1 {
                fps: FpsPolicy::Fixed(24),
                ..base.clone()
            },
            ProxyProfileV1 {
                codec: ProxyCodec::H264Hardware,
                ..base.clone()
            },
            ProxyProfileV1 {
                jpeg_quality: 9,
                ..base.clone()
            },
            ProxyProfileV1 {
                audio: ProxyAudio::None,
                ..base.clone()
            },
        ];
        for v in variants {
            assert_ne!(v.cache_fragment(), f0);
        }
    }
}
