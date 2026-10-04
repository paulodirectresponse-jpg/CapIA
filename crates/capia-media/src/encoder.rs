//! `EncoderCapability` (ADR-067): o que este FFmpeg consegue codificar **e** o que a política do
//! projeto permite usar. Detecção = lista compilada (`-encoders`) × teste real de 2 quadros (um
//! encoder compilado pode não ter hardware/driver). `libx264`/`libx265` (GPL) e qualquer encoder
//! fora do catálogo são **proibidos**: nunca selecionados e nunca substituídos em silêncio.

use crate::error::{MediaError, MediaErrorCode};
use crate::process::{StreamLimits, run_collect};
use crate::proxy::list_encoders;
use crate::toolchain::MediaToolchain;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderBackend {
    Nvenc,
    Qsv,
    Amf,
    MediaFoundation,
    OpenH264,
    VideoToolbox,
    Vaapi,
    V4l2,
    /// MPEG-4 Part 2 nativo do FFmpeg (LGPL): **não é H.264**; só como referência explícita.
    NativeMpeg4,
    /// x264/x265: GPL, proibido (ADR-032).
    GplX26x,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderPolicy {
    /// Licença do encoder compatível com a política (LGPL/hardware/SO).
    Approved,
    /// Permitido tecnicamente, mas com condição jurídica a decidir antes de distribuir
    /// (OpenH264: o binário da Cisco cobre a patente; compilar do fonte não).
    ApprovedWithConditions,
    Prohibited,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncoderCapability {
    pub backend: EncoderBackend,
    pub codec: String,
    pub ffmpeg_name: String,
    pub hardware: bool,
    pub policy: EncoderPolicy,
    /// Aparece em `ffmpeg -encoders`.
    pub compiled_in: bool,
    /// Compilado, permitido **e** o teste real de codificação passou.
    pub available: bool,
    pub reason_unavailable: Option<String>,
    pub pixel_formats: Vec<String>,
    /// Perfis que o catálogo declara (selecionáveis com `-profile:v`).
    pub profiles: Vec<String>,
    pub max_level: Option<String>,
    pub note: Option<String>,
}

struct CatalogEntry {
    name: &'static str,
    backend: EncoderBackend,
    codec: &'static str,
    hardware: bool,
    policy: EncoderPolicy,
    profiles: &'static [&'static str],
    note: &'static str,
}

const H264_PROFILES: &[&str] = &["baseline", "main", "high"];

/// Catálogo completo (ordem = preferência dentro do H.264). Fora daqui ⇒ proibido.
const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "h264_nvenc",
        backend: EncoderBackend::Nvenc,
        codec: "h264",
        hardware: true,
        policy: EncoderPolicy::Approved,
        profiles: H264_PROFILES,
        note: "NVIDIA NVENC (GPU)",
    },
    CatalogEntry {
        name: "h264_qsv",
        backend: EncoderBackend::Qsv,
        codec: "h264",
        hardware: true,
        policy: EncoderPolicy::Approved,
        profiles: H264_PROFILES,
        note: "Intel Quick Sync (GPU)",
    },
    CatalogEntry {
        name: "h264_amf",
        backend: EncoderBackend::Amf,
        codec: "h264",
        hardware: true,
        policy: EncoderPolicy::Approved,
        profiles: H264_PROFILES,
        note: "AMD AMF (GPU)",
    },
    CatalogEntry {
        name: "h264_videotoolbox",
        backend: EncoderBackend::VideoToolbox,
        codec: "h264",
        hardware: true,
        policy: EncoderPolicy::Approved,
        profiles: H264_PROFILES,
        note: "Apple VideoToolbox",
    },
    CatalogEntry {
        name: "h264_vaapi",
        backend: EncoderBackend::Vaapi,
        codec: "h264",
        hardware: true,
        policy: EncoderPolicy::Approved,
        profiles: H264_PROFILES,
        note: "VA-API (Linux, GPU)",
    },
    CatalogEntry {
        name: "h264_v4l2m2m",
        backend: EncoderBackend::V4l2,
        codec: "h264",
        hardware: true,
        policy: EncoderPolicy::Approved,
        profiles: H264_PROFILES,
        note: "V4L2 mem2mem (Linux, SoC)",
    },
    CatalogEntry {
        name: "h264_mf",
        backend: EncoderBackend::MediaFoundation,
        codec: "h264",
        hardware: false,
        policy: EncoderPolicy::Approved,
        profiles: H264_PROFILES,
        note: "Windows Media Foundation (MFT do sistema; hardware ou software conforme o driver)",
    },
    CatalogEntry {
        name: "libopenh264",
        backend: EncoderBackend::OpenH264,
        codec: "h264",
        hardware: false,
        policy: EncoderPolicy::ApprovedWithConditions,
        profiles: &["baseline", "main"],
        note: "OpenH264 (BSD): a cobertura de patentes exige o binário da Cisco — decisão jurídica pendente",
    },
    CatalogEntry {
        name: "hevc_nvenc",
        backend: EncoderBackend::Nvenc,
        codec: "hevc",
        hardware: true,
        policy: EncoderPolicy::Approved,
        profiles: &["main"],
        note: "NVIDIA NVENC HEVC (não usado no export da Fase 2)",
    },
    CatalogEntry {
        name: "mpeg4",
        backend: EncoderBackend::NativeMpeg4,
        codec: "mpeg4",
        hardware: false,
        policy: EncoderPolicy::Approved,
        profiles: &[],
        note: "MPEG-4 Part 2 nativo (LGPL). NÃO é H.264: referência de mux/sincronismo, só pedida de forma explícita",
    },
    CatalogEntry {
        name: "libx264",
        backend: EncoderBackend::GplX26x,
        codec: "h264",
        hardware: false,
        policy: EncoderPolicy::Prohibited,
        profiles: &[],
        note: "GPL — proibido (ADR-032)",
    },
    CatalogEntry {
        name: "libx264rgb",
        backend: EncoderBackend::GplX26x,
        codec: "h264",
        hardware: false,
        policy: EncoderPolicy::Prohibited,
        profiles: &[],
        note: "GPL — proibido (ADR-032)",
    },
    CatalogEntry {
        name: "libx265",
        backend: EncoderBackend::GplX26x,
        codec: "hevc",
        hardware: false,
        policy: EncoderPolicy::Prohibited,
        profiles: &[],
        note: "GPL — proibido (ADR-032)",
    },
];

/// Política de um encoder pelo nome do FFmpeg: `None` ⇒ fora do catálogo (proibido por omissão).
pub fn encoder_policy(name: &str) -> Option<EncoderPolicy> {
    CATALOG.iter().find(|e| e.name == name).map(|e| e.policy)
}

/// Qual codec de vídeo o export pede.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportCodec {
    H264,
    /// Referência explícita (não H.264): prova mux/sincronismo onde não há encoder H.264 aprovado.
    Mpeg4Reference,
}

impl ExportCodec {
    pub fn codec_name(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Mpeg4Reference => "mpeg4",
        }
    }
}

/// A causa útil do erro do ffmpeg: ignora as frases genéricas de fim ("Nothing was written…").
fn first_line(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let generic = |l: &str| {
        l.contains("Nothing was written")
            || l.contains("Conversion failed")
            || l.contains("Error while opening encoder")
            || l.contains("Error initializing output stream")
            || l.contains("Task finished with error")
    };
    let useful: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !generic(l))
        .collect();
    let key = [
        "annot",
        "ailed",
        "o device",
        "nknown",
        "not found",
        "rror",
        "nsupported",
    ];
    let line = useful
        .iter()
        .find(|l| key.iter().any(|k| l.contains(k)))
        .or_else(|| useful.last())
        .copied()
        .unwrap_or("encoder test failed");
    // tira o prefixo `[componente @ 0x…]`, que muda a cada execução
    let line = line.rsplit_once("] ").map_or(line, |(_, r)| r);
    line.chars().take(240).collect()
}

fn probe_encoder(tc: &MediaToolchain, name: &str) -> Result<Vec<String>, String> {
    let Some(ffmpeg) = tc.ffmpeg.as_deref() else {
        return Err("ffmpeg was not found".into());
    };
    let mut args: Vec<OsString> = [
        "-hide_banner",
        "-v",
        "error",
        "-nostdin",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=640x360:r=30",
        "-frames:v",
        "2",
        "-pix_fmt",
        "yuv420p",
        "-c:v",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(name.into());
    for a in ["-f", "null", "-"] {
        args.push(a.into());
    }
    match run_collect(
        ffmpeg,
        &args,
        &StreamLimits::new(Duration::from_secs(30)),
        1 << 16,
        &|| false,
    ) {
        Ok(o) if o.status.success() => Ok(pixel_formats(tc, name)),
        Ok(o) => Err(first_line(&o.stderr)),
        Err(e) => Err(e.message),
    }
}

fn pixel_formats(tc: &MediaToolchain, name: &str) -> Vec<String> {
    let Some(ffmpeg) = tc.ffmpeg.as_deref() else {
        return Vec::new();
    };
    let args: Vec<OsString> = [
        "-hide_banner".into(),
        "-v".into(),
        "error".into(),
        "-h".into(),
        format!("encoder={name}"),
    ]
    .iter()
    .map(OsString::from)
    .collect();
    let Ok(out) = run_collect(
        ffmpeg,
        &args,
        &StreamLimits::new(Duration::from_secs(15)),
        1 << 18,
        &|| false,
    ) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.trim().strip_prefix("Supported pixel formats:"))
        .map(|r| r.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Resultado da detecção: uma entrada por item do catálogo (compilado ou não), ordenado por
/// preferência. Executa um teste real de codificação por encoder compilado e permitido.
pub fn detect_encoders(tc: &MediaToolchain) -> Result<Vec<EncoderCapability>, MediaError> {
    let compiled = list_encoders(tc)?;
    Ok(CATALOG
        .iter()
        .map(|e| {
            let compiled_in = compiled.iter().any(|c| c == e.name);
            let (available, reason, pix) = if e.policy == EncoderPolicy::Prohibited {
                (
                    false,
                    Some(format!(
                        "{} is prohibited by policy (GPL; ADR-032){}",
                        e.name,
                        if compiled_in {
                            " — present in this FFmpeg build but never used"
                        } else {
                            ""
                        }
                    )),
                    Vec::new(),
                )
            } else if e.name == "h264_vaapi" && compiled_in {
                (
                    false,
                    Some(
                        "VA-API needs a hardware-upload pipeline (-vaapi_device + hwupload), \
                         which the Phase 2 export does not implement"
                            .to_owned(),
                    ),
                    Vec::new(),
                )
            } else if !compiled_in {
                (
                    false,
                    Some("not compiled into this FFmpeg build".to_owned()),
                    Vec::new(),
                )
            } else {
                match probe_encoder(tc, e.name) {
                    Ok(p) => (true, None, p),
                    Err(why) => (false, Some(why), Vec::new()),
                }
            };
            EncoderCapability {
                backend: e.backend,
                codec: e.codec.to_owned(),
                ffmpeg_name: e.name.to_owned(),
                hardware: e.hardware,
                policy: e.policy,
                compiled_in,
                available,
                reason_unavailable: reason,
                pixel_formats: pix,
                profiles: e.profiles.iter().map(|s| (*s).to_owned()).collect(),
                max_level: None,
                note: Some(e.note.to_owned()),
            }
        })
        .collect())
}

/// Escolhe o encoder do export. `preferred` (nome do FFmpeg) é obedecido **ou** falha: proibido ⇒
/// `MEDIA_ENCODER_PROHIBITED`; indisponível ⇒ `MEDIA_ENCODER_UNAVAILABLE` com o motivo. Sem
/// `preferred`, o primeiro disponível e aprovado do codec; nunca cai para outro codec.
pub fn select_export_encoder<'a>(
    caps: &'a [EncoderCapability],
    codec: ExportCodec,
    preferred: Option<&str>,
) -> Result<&'a EncoderCapability, MediaError> {
    if let Some(name) = preferred {
        match encoder_policy(name) {
            None | Some(EncoderPolicy::Prohibited) => {
                return Err(MediaError::new(
                    MediaErrorCode::MediaEncoderProhibited,
                    format!(
                        "encoder `{name}` is {} and is never used",
                        if encoder_policy(name).is_none() {
                            "not in the approved catalog"
                        } else {
                            "GPL-licensed (ADR-032)"
                        }
                    ),
                ));
            }
            Some(_) => {}
        }
        let cap = caps.iter().find(|c| c.ffmpeg_name == name).ok_or_else(|| {
            MediaError::new(
                MediaErrorCode::MediaEncoderUnavailable,
                format!("encoder `{name}` was not detected"),
            )
        })?;
        if cap.codec != codec.codec_name() {
            return Err(MediaError::new(
                MediaErrorCode::MediaEncoderUnavailable,
                format!(
                    "encoder `{name}` produces {}, not {}",
                    cap.codec,
                    codec.codec_name()
                ),
            ));
        }
        return if cap.available {
            Ok(cap)
        } else {
            Err(MediaError::new(
                MediaErrorCode::MediaEncoderUnavailable,
                format!(
                    "encoder `{name}` is unavailable: {}",
                    cap.reason_unavailable.as_deref().unwrap_or("unknown")
                ),
            ))
        };
    }
    caps.iter()
        .find(|c| {
            c.codec == codec.codec_name() && c.available && c.policy != EncoderPolicy::Prohibited
        })
        .ok_or_else(|| {
            let tried: Vec<String> = caps
                .iter()
                .filter(|c| c.codec == codec.codec_name())
                .map(|c| {
                    format!(
                        "{}: {}",
                        c.ffmpeg_name,
                        c.reason_unavailable.as_deref().unwrap_or("not selected")
                    )
                })
                .collect();
            MediaError::new(
                MediaErrorCode::MediaEncoderUnavailable,
                format!(
                    "no approved {} encoder is available on this machine ({})",
                    codec.codec_name(),
                    tried.join("; ")
                ),
            )
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn cap(name: &str, available: bool) -> EncoderCapability {
        let e = CATALOG.iter().find(|e| e.name == name).unwrap();
        EncoderCapability {
            backend: e.backend,
            codec: e.codec.into(),
            ffmpeg_name: name.into(),
            hardware: e.hardware,
            policy: e.policy,
            compiled_in: true,
            available,
            reason_unavailable: (!available).then(|| "no device".to_owned()),
            pixel_formats: vec![],
            profiles: vec![],
            max_level: None,
            note: None,
        }
    }

    #[test]
    fn gpl_encoders_are_prohibited_and_unknown_ones_too() {
        for name in ["libx264", "libx264rgb", "libx265", "libfoo", "h264_made_up"] {
            assert!(
                matches!(encoder_policy(name), None | Some(EncoderPolicy::Prohibited)),
                "{name}"
            );
        }
        let caps = vec![cap("libx264", true), cap("h264_nvenc", true)];
        for name in ["libx264", "libx265", "totally_unknown"] {
            let e = select_export_encoder(&caps, ExportCodec::H264, Some(name)).unwrap_err();
            assert_eq!(e.code, MediaErrorCode::MediaEncoderProhibited, "{name}");
        }
    }

    #[test]
    fn automatic_choice_never_picks_a_prohibited_or_unavailable_encoder() {
        // libx264 "disponível" e à frente na lista: ainda assim nunca é escolhido
        let caps = vec![
            cap("libx264", true),
            cap("h264_nvenc", false),
            cap("h264_mf", true),
        ];
        assert_eq!(
            select_export_encoder(&caps, ExportCodec::H264, None)
                .unwrap()
                .ffmpeg_name,
            "h264_mf"
        );
        let only_gpl = vec![cap("libx264", true), cap("h264_nvenc", false)];
        let e = select_export_encoder(&only_gpl, ExportCodec::H264, None).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaEncoderUnavailable);
        assert!(e.message.contains("h264_nvenc: no device"), "{}", e.message);
    }

    #[test]
    fn there_is_no_silent_fallback_between_codecs_or_encoders() {
        let caps = vec![
            cap("h264_nvenc", false),
            cap("mpeg4", true),
            cap("h264_mf", true),
        ];
        // pedido explícito de um encoder indisponível falha em vez de trocar por outro
        let e = select_export_encoder(&caps, ExportCodec::H264, Some("h264_nvenc")).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaEncoderUnavailable);
        // H.264 sem encoder aprovado não vira MPEG-4
        let only_mpeg4 = vec![cap("mpeg4", true)];
        assert!(select_export_encoder(&only_mpeg4, ExportCodec::H264, None).is_err());
        assert_eq!(
            select_export_encoder(&only_mpeg4, ExportCodec::Mpeg4Reference, None)
                .unwrap()
                .ffmpeg_name,
            "mpeg4"
        );
        // um encoder de outro codec não atende o pedido
        let e = select_export_encoder(&caps, ExportCodec::H264, Some("mpeg4")).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaEncoderUnavailable);
    }

    #[test]
    fn catalog_is_consistent() {
        assert!(
            CATALOG
                .iter()
                .filter(|e| e.name.contains("x26"))
                .all(|e| e.policy == EncoderPolicy::Prohibited)
        );
        let mut names: Vec<_> = CATALOG.iter().map(|e| e.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), CATALOG.len());
        for e in CATALOG {
            assert_eq!(
                e.policy == EncoderPolicy::Prohibited,
                e.backend == EncoderBackend::GplX26x
            );
        }
    }
}
