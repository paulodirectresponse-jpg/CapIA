//! ffprobe JSON → [`MediaInfo`] (ADR-047 §5). Entrada **hostil**: nunca pânico, nunca overflow,
//! nunca float no resultado. Campos inválidos viram `None` + aviso determinístico; estruturas
//! inutilizáveis viram erro estruturado.

use crate::error::{MediaError, MediaErrorCode};
use crate::info::{
    AudioStream, ContainerInfo, MediaInfo, MediaKind, OtherStream, StreamInfo, VideoStream,
};
use crate::limits::*;
use capia_time::{Rational, TICKS_PER_SECOND, Ticks};
use serde_json::Value;

/// Demuxers que leem **outros** arquivos/rede a partir do conteúdo (playlists): rejeitados.
const DENIED_FORMATS: &[&str] = &[
    "hls",
    "applehttp",
    "concat",
    "ffconcat",
    "dash",
    "sdp",
    "rtsp",
    "rtp",
    "lavfi",
    "tee",
];

fn invalid(msg: impl Into<String>) -> MediaError {
    MediaError::new(MediaErrorCode::MediaMetadataInvalid, msg)
}

/// Decimal textual (`"1.234000"`) → `Ticks` por **aritmética inteira**, arredondando half-up.
/// Rejeita sinal, expoente, `N/A`, vazio e valores acima de [`MAX_MEDIA_SECONDS`].
pub fn parse_decimal_ticks(s: &str) -> Option<Ticks> {
    let s = s.trim();
    if s.is_empty() || s.len() > 48 {
        return None;
    }
    let (int_s, frac_s) = s.split_once('.').unwrap_or((s, ""));
    if (int_s.is_empty() && frac_s.is_empty())
        || !int_s.bytes().all(|b| b.is_ascii_digit())
        || !frac_s.bytes().all(|b| b.is_ascii_digit())
        || int_s.len() > 12
    {
        return None;
    }
    let int: i128 = if int_s.is_empty() {
        0
    } else {
        int_s.parse().ok()?
    };
    let mut frac_digits: String = frac_s.chars().take(9).collect();
    while frac_digits.len() < 9 {
        frac_digits.push('0');
    }
    let frac: i128 = frac_digits.parse().ok()?;
    let tps = i128::from(TICKS_PER_SECOND);
    let ticks = int
        .checked_mul(tps)?
        .checked_add((frac * tps + 500_000_000) / 1_000_000_000)?;
    if ticks > i128::from(MAX_MEDIA_SECONDS) * tps {
        return None;
    }
    i64::try_from(ticks).ok().map(Ticks)
}

/// `"30000/1001"` / `"25"` → `Rational` reduzido e plausível (`0 < fps ≤ MAX_FPS`). `0/0` e
/// qualquer valor inválido (denominador 0, negativo, fora de faixa) ⇒ `None`.
pub fn parse_frame_rate(s: &str) -> Option<Rational> {
    let r = parse_ratio(s, '/')?;
    (r.is_positive() && r <= Rational::from_int(MAX_FPS)).then_some(r)
}

fn parse_ratio(s: &str, sep: char) -> Option<Rational> {
    let s = s.trim();
    if s.is_empty() || s.len() > 40 {
        return None;
    }
    let (n, d) = s.split_once(sep).unwrap_or((s, "1"));
    let n: i64 = n.trim().parse().ok()?;
    let d: i64 = d.trim().parse().ok()?;
    if d <= 0 {
        return None;
    }
    Rational::new(n, d).ok()
}

fn sanitize(s: &str, max: usize) -> String {
    s.chars()
        .take(max)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '+' | ' ') {
                c
            } else {
                '?'
            }
        })
        .collect()
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// Inteiro sem sinal que o ffprobe ora emite como número, ora como string.
fn uint(v: &Value, key: &str) -> Option<u64> {
    match v.get(key)? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn alpha_of(pix_fmt: &str) -> Option<bool> {
    const WITH: &[&str] = &[
        "yuva", "gbrap", "rgba", "bgra", "argb", "abgr", "ya", "ayuv",
    ];
    const WITHOUT: &[&str] = &[
        "yuv", "yuvj", "gray", "rgb", "bgr", "gbrp", "nv12", "nv21", "p010", "monow", "monob",
    ];
    if WITH.iter().any(|p| pix_fmt.starts_with(p)) {
        Some(true)
    } else if WITHOUT.iter().any(|p| pix_fmt.starts_with(p)) {
        Some(false)
    } else {
        None
    }
}

/// Rotação horária (0/90/180/270) a partir de `tags.rotate` ou do Display Matrix.
fn rotation_of(stream: &Value, warnings: &mut Vec<String>, index: u32) -> u32 {
    let mut cw: Option<i64> = None;
    if let Some(r) = stream
        .get("tags")
        .and_then(|t| text(t, "rotate"))
        .and_then(|s| s.trim().parse::<i64>().ok())
    {
        cw = Some(r);
    }
    if let Some(list) = stream.get("side_data_list").and_then(Value::as_array) {
        for sd in list.iter().take(16) {
            // o Display Matrix informa graus anti-horários; a UI quer horário
            if let Some(f) = sd.get("rotation").and_then(Value::as_f64)
                && f.is_finite()
                && f.abs() < 1.0e6
            {
                #[allow(clippy::cast_possible_truncation)]
                let r = f.round() as i64;
                cw = Some(-r);
            }
        }
    }
    let Some(r) = cw else { return 0 };
    let m = r.rem_euclid(360);
    if m % 90 == 0 {
        // 0, 90, 180, 270
        u32::try_from(m).unwrap_or(0)
    } else {
        warnings.push(format!("stream {index}: unsupported rotation {r}"));
        0
    }
}

fn duration_of(v: &Value, key: &str, warnings: &mut Vec<String>, what: &str) -> Option<Ticks> {
    let s = match v.get(key)? {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    if s == "N/A" {
        return None;
    }
    let t = parse_decimal_ticks(&s);
    if t.is_none() {
        warnings.push(format!("{what}: invalid or implausible duration"));
    }
    t
}

fn parse_stream(s: &Value, warnings: &mut Vec<String>) -> Result<StreamInfo, MediaError> {
    let index = uint(s, "index")
        .and_then(|i| u32::try_from(i).ok())
        .ok_or_else(|| invalid("stream without a valid index"))?;
    let codec_type = sanitize(text(s, "codec_type").unwrap_or("unknown"), 32);
    let codec = text(s, "codec_name").map(|c| sanitize(c, 64));
    let what = format!("stream {index}");
    let bit_rate = uint(s, "bit_rate");
    let duration = duration_of(s, "duration", warnings, &what);
    let other = |warnings: &mut Vec<String>, why: &str| {
        warnings.push(format!("stream {index}: {why}"));
        StreamInfo::Other(OtherStream {
            index,
            codec_type: codec_type.clone(),
            codec: codec.clone(),
        })
    };
    match codec_type.as_str() {
        "video" => {
            let w = uint(s, "width").and_then(|v| u32::try_from(v).ok());
            let h = uint(s, "height").and_then(|v| u32::try_from(v).ok());
            let (Some(width), Some(height)) = (w, h) else {
                return Ok(other(warnings, "video without dimensions"));
            };
            if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
                return Ok(other(warnings, "implausible video dimensions"));
            }
            let pixel_format = text(s, "pix_fmt").map(|p| sanitize(p, 32));
            let attached = s
                .get("disposition")
                .and_then(|d| d.get("attached_pic"))
                .and_then(Value::as_u64)
                == Some(1);
            let avg = text(s, "avg_frame_rate").and_then(parse_frame_rate);
            let nominal = text(s, "r_frame_rate").and_then(parse_frame_rate);
            Ok(StreamInfo::Video(VideoStream {
                index,
                codec: codec.unwrap_or_else(|| "unknown".into()),
                width,
                height,
                pixel_aspect: text(s, "sample_aspect_ratio")
                    .and_then(|r| parse_ratio(r, ':'))
                    .filter(Rational::is_positive),
                display_aspect: text(s, "display_aspect_ratio")
                    .and_then(|r| parse_ratio(r, ':'))
                    .filter(Rational::is_positive),
                frame_rate: avg.or(nominal),
                nominal_frame_rate: nominal,
                time_base: text(s, "time_base")
                    .and_then(|r| parse_ratio(r, '/'))
                    .filter(Rational::is_positive),
                has_alpha: pixel_format.as_deref().and_then(alpha_of),
                pixel_format,
                rotation: rotation_of(s, warnings, index),
                bit_rate,
                duration,
                frames: uint(s, "nb_frames"),
                attached_picture: attached,
            }))
        }
        "audio" => {
            let channels = uint(s, "channels").and_then(|v| u32::try_from(v).ok());
            let rate = uint(s, "sample_rate").and_then(|v| u32::try_from(v).ok());
            match (channels, rate) {
                (Some(c), Some(r))
                    if (1..=MAX_CHANNELS).contains(&c) && (1..=MAX_SAMPLE_RATE).contains(&r) =>
                {
                    Ok(StreamInfo::Audio(AudioStream {
                        index,
                        codec: codec.unwrap_or_else(|| "unknown".into()),
                        channels: c,
                        sample_rate: r,
                        bit_rate,
                        duration,
                    }))
                }
                _ => Ok(other(
                    warnings,
                    "audio with implausible channels/sample rate",
                )),
            }
        }
        _ => Ok(StreamInfo::Other(OtherStream {
            index,
            codec_type,
            codec,
        })),
    }
}

fn is_still_image_container(formats: &[String]) -> bool {
    formats
        .iter()
        .any(|f| f == "image2" || f.ends_with("_pipe") || f == "ico" || f == "image_jpeg")
}

/// Normaliza a saída JSON do ffprobe (`-show_format -show_streams`).
pub fn normalize_ffprobe_json(json: &[u8]) -> Result<MediaInfo, MediaError> {
    let root: Value = serde_json::from_slice(json)
        .map_err(|e| invalid(format!("ffprobe output is not valid JSON: {e}")))?;
    let Some(format) = root.get("format").filter(|f| f.is_object()) else {
        return Err(invalid("ffprobe output has no `format` object"));
    };
    let raw_streams: &[Value] = root
        .get("streams")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    if raw_streams.len() > MAX_STREAMS {
        return Err(MediaError::new(
            MediaErrorCode::MediaUnsupportedFormat,
            format!("too many streams ({} > {MAX_STREAMS})", raw_streams.len()),
        ));
    }

    let mut formats: Vec<String> = text(format, "format_name")
        .unwrap_or("")
        .split(',')
        .map(|f| sanitize(f.trim(), 32))
        .filter(|f| !f.is_empty())
        .take(16)
        .collect();
    formats.sort();
    formats.dedup();
    if let Some(bad) = formats
        .iter()
        .find(|f| DENIED_FORMATS.contains(&f.as_str()))
    {
        return Err(MediaError::new(
            MediaErrorCode::MediaUnsupportedFormat,
            format!("container format `{bad}` is not accepted (playlist/network demuxer)"),
        ));
    }

    let mut warnings = Vec::new();
    let mut streams = Vec::with_capacity(raw_streams.len());
    for s in raw_streams {
        streams.push(parse_stream(s, &mut warnings)?);
    }
    streams.sort_by_key(StreamInfo::index);
    if streams.windows(2).any(|w| w[0].index() == w[1].index()) {
        return Err(invalid("duplicate stream index"));
    }

    let default_video = streams.iter().find_map(|s| match s {
        StreamInfo::Video(v) if !v.attached_picture => Some(v.index),
        _ => None,
    });
    let default_audio = streams.iter().find_map(|s| match s {
        StreamInfo::Audio(a) => Some(a.index),
        _ => None,
    });

    let kind = match (default_video, default_audio) {
        (Some(_), None) if is_still_image_container(&formats) => MediaKind::Image,
        (Some(_), _) => MediaKind::Video,
        (None, Some(_)) => MediaKind::Audio,
        (None, None) => {
            return Err(if warnings.is_empty() {
                MediaError::new(
                    MediaErrorCode::MediaUnsupportedFormat,
                    "no video, audio or image stream",
                )
            } else {
                invalid(format!("no usable stream: {}", warnings.join("; ")))
            });
        }
    };

    let container_duration = duration_of(format, "duration", &mut warnings, "container");
    let stream_duration = |idx: Option<u32>| {
        streams.iter().find_map(|s| match (s, idx) {
            (StreamInfo::Video(v), Some(i)) if v.index == i => v.duration,
            (StreamInfo::Audio(a), Some(i)) if a.index == i => a.duration,
            _ => None,
        })
    };
    let duration = match kind {
        MediaKind::Image => None,
        _ => container_duration.or_else(|| {
            stream_duration(default_video)
                .into_iter()
                .chain(stream_duration(default_audio))
                .max()
        }),
    };
    if kind != MediaKind::Image && duration.is_none() {
        return Err(invalid("missing or implausible duration"));
    }

    if kind == MediaKind::Image {
        for s in &mut streams {
            if let StreamInfo::Video(v) = s {
                v.frame_rate = None;
                v.nominal_frame_rate = None;
            }
        }
    }

    Ok(MediaInfo {
        kind,
        container: ContainerInfo {
            formats,
            duration: container_duration,
            bit_rate: uint(format, "bit_rate"),
            stream_count: u32::try_from(streams.len()).unwrap_or(u32::MAX),
        },
        streams,
        default_video,
        default_audio,
        duration,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn decimal_durations_are_exact_integer_ticks() {
        assert_eq!(
            parse_decimal_ticks("1.000000"),
            Some(Ticks(TICKS_PER_SECOND))
        );
        assert_eq!(
            parse_decimal_ticks("0.5"),
            Some(Ticks(TICKS_PER_SECOND / 2))
        );
        assert_eq!(
            parse_decimal_ticks("12"),
            Some(Ticks(12 * TICKS_PER_SECOND))
        );
        // 1/3 s com 9 casas: arredonda half-up no tick
        assert_eq!(parse_decimal_ticks("0.333333333"), Some(Ticks(235_200_000)));
        assert_eq!(
            parse_decimal_ticks(".25"),
            Some(Ticks(TICKS_PER_SECOND / 4))
        );
    }

    #[test]
    fn hostile_decimals_are_rejected_without_panic() {
        for bad in [
            "",
            " ",
            ".",
            "-1",
            "+1",
            "1e9",
            "nan",
            "inf",
            "NaN",
            "N/A",
            "1.2.3",
            "0x10",
            "99999999999999999999",
            "4000000",
            "1000000000000",
            "1,5",
            "１２",
        ] {
            assert_eq!(parse_decimal_ticks(bad), None, "{bad:?}");
        }
        // o limite exato (1.000 h) é aceito; um tick a mais não
        assert!(parse_decimal_ticks("3600000").is_some());
        assert_eq!(parse_decimal_ticks("3600000.000000001"), None);
    }

    #[test]
    fn frame_rates() {
        assert_eq!(
            parse_frame_rate("30000/1001"),
            Rational::new(30000, 1001).ok()
        );
        assert_eq!(parse_frame_rate("25"), Some(Rational::from_int(25)));
        assert_eq!(parse_frame_rate("50/2"), Some(Rational::from_int(25)));
        for bad in [
            "0/0",
            "1/0",
            "-25/1",
            "0/1",
            "25/-1",
            "x",
            "",
            "1000000/1",
            "9223372036854775807/1",
            "1/9223372036854775808",
        ] {
            assert_eq!(parse_frame_rate(bad), None, "{bad:?}");
        }
    }

    fn doc(streams: &str, format: &str) -> Vec<u8> {
        format!(r#"{{"streams":[{streams}],"format":{format}}}"#).into_bytes()
    }

    const VIDEO: &str = r#"{"index":0,"codec_type":"video","codec_name":"h264","width":64,"height":48,"pix_fmt":"yuv420p","avg_frame_rate":"25/1","r_frame_rate":"25/1","time_base":"1/12800","sample_aspect_ratio":"1:1","display_aspect_ratio":"4:3"}"#;
    const AUDIO: &str =
        r#"{"index":1,"codec_type":"audio","codec_name":"aac","channels":2,"sample_rate":"48000"}"#;
    const MP4: &str = r#"{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"1.000000"}"#;

    #[test]
    fn video_with_audio_has_explicit_defaults() {
        let m = normalize_ffprobe_json(&doc(&format!("{AUDIO},{VIDEO}"), MP4)).unwrap();
        assert_eq!(m.kind, MediaKind::Video);
        // streams ordenados por índice mesmo que a entrada venha embaralhada
        assert_eq!(
            m.streams.iter().map(StreamInfo::index).collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!((m.default_video, m.default_audio), (Some(0), Some(1)));
        assert_eq!(m.duration, Some(Ticks(TICKS_PER_SECOND)));
        assert_eq!(
            m.container.formats,
            ["3g2", "3gp", "m4a", "mj2", "mov", "mp4"]
        );
        assert_eq!(m.video().unwrap().has_alpha, Some(false));
    }

    #[test]
    fn first_video_wins_and_attached_picture_is_not_video() {
        let cover = r#"{"index":0,"codec_type":"video","codec_name":"mjpeg","width":10,"height":10,"disposition":{"attached_pic":1}}"#;
        let m = normalize_ffprobe_json(&doc(
            &format!("{cover},{AUDIO}"),
            r#"{"format_name":"mp3","duration":"2"}"#,
        ))
        .unwrap();
        assert_eq!(
            (m.kind, m.default_video, m.default_audio),
            (MediaKind::Audio, None, Some(1))
        );
        let v2 = VIDEO.replace(r#""index":0"#, r#""index":3"#);
        let m = normalize_ffprobe_json(&doc(&format!("{VIDEO},{v2},{AUDIO}"), MP4)).unwrap();
        assert_eq!(m.default_video, Some(0));
    }

    #[test]
    fn images_have_no_duration_and_know_alpha_only_when_declared() {
        let png = r#"{"index":0,"codec_type":"video","codec_name":"png","width":32,"height":24,"pix_fmt":"rgba","r_frame_rate":"25/1","avg_frame_rate":"25/1"}"#;
        let m = normalize_ffprobe_json(&doc(png, r#"{"format_name":"png_pipe"}"#)).unwrap();
        assert_eq!(m.kind, MediaKind::Image);
        assert_eq!(m.duration, None);
        assert_eq!(m.video().unwrap().has_alpha, Some(true));
        assert_eq!(m.video().unwrap().frame_rate, None);
        let pal = png.replace("rgba", "pal8");
        let m = normalize_ffprobe_json(&doc(&pal, r#"{"format_name":"png_pipe"}"#)).unwrap();
        assert_eq!(m.video().unwrap().has_alpha, None);
    }

    #[test]
    fn absurd_values_are_dropped_or_rejected_never_trusted() {
        // dimensão absurda ⇒ stream inutilizável ⇒ erro estruturado
        let huge = VIDEO.replace(r#""width":64"#, r#""width":4294967295"#);
        let e = normalize_ffprobe_json(&doc(&huge, MP4)).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaMetadataInvalid);
        let zero = VIDEO.replace(r#""height":48"#, r#""height":0"#);
        assert_eq!(
            normalize_ffprobe_json(&doc(&zero, MP4)).unwrap_err().code,
            MediaErrorCode::MediaMetadataInvalid
        );
        // duração absurda ⇒ sem duração ⇒ erro (vídeo exige duração)
        let e = normalize_ffprobe_json(&doc(
            VIDEO,
            r#"{"format_name":"mp4","duration":"99999999999"}"#,
        ))
        .unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaMetadataInvalid);
        // frame rate inválido não derruba o arquivo: vira None
        let bad_fps = VIDEO
            .replace(r#""avg_frame_rate":"25/1""#, r#""avg_frame_rate":"1/0""#)
            .replace(r#""r_frame_rate":"25/1""#, r#""r_frame_rate":"0/0""#);
        let m = normalize_ffprobe_json(&doc(&bad_fps, MP4)).unwrap();
        assert_eq!(m.video().unwrap().frame_rate, None);
        // canais/sample rate absurdos ⇒ stream de áudio descartado com aviso
        let bad_audio = AUDIO.replace(r#""channels":2"#, r#""channels":9999"#);
        let m = normalize_ffprobe_json(&doc(&format!("{VIDEO},{bad_audio}"), MP4)).unwrap();
        assert_eq!(m.default_audio, None);
        assert_eq!(m.warnings.len(), 1);
    }

    #[test]
    fn malformed_probe_output_is_a_structured_error() {
        for bad in [
            &b""[..],
            b"{",
            b"null",
            b"[]",
            b"{}",
            br#"{"format":1}"#,
            br#"{"format":{},"streams":"x"}"#,
            br#"{"format":{"format_name":"mp4"},"streams":[{"codec_type":"video"}]}"#,
            br#"{"format":{"format_name":"mp4"},"streams":[]}"#,
            &[0xff, 0xfe, 0x00],
        ] {
            let e = normalize_ffprobe_json(bad).unwrap_err();
            assert!(
                matches!(
                    e.code,
                    MediaErrorCode::MediaMetadataInvalid | MediaErrorCode::MediaUnsupportedFormat
                ),
                "{bad:?} -> {e}"
            );
        }
    }

    #[test]
    fn playlist_and_network_demuxers_are_denied() {
        for f in ["hls,applehttp", "concat", "dash", "sdp", "rtsp", "lavfi"] {
            let e = normalize_ffprobe_json(&doc(
                VIDEO,
                &format!(r#"{{"format_name":"{f}","duration":"1"}}"#),
            ))
            .unwrap_err();
            assert_eq!(e.code, MediaErrorCode::MediaUnsupportedFormat, "{f}");
        }
    }

    #[test]
    fn rotation_from_display_matrix_is_clockwise_and_validated() {
        let rot = |extra: &str| {
            let v = VIDEO.replace(r#""time_base""#, &format!("{extra},\"time_base\""));
            normalize_ffprobe_json(&doc(&v, MP4))
                .unwrap()
                .video()
                .unwrap()
                .rotation
        };
        assert_eq!(
            rot(r#""side_data_list":[{"side_data_type":"Display Matrix","rotation":-90}]"#),
            90
        );
        assert_eq!(rot(r#""side_data_list":[{"rotation":90}]"#), 270);
        assert_eq!(rot(r#""tags":{"rotate":"180"}"#), 180);
        assert_eq!(rot(r#""side_data_list":[{"rotation":37.5}]"#), 0); // não-múltiplo ⇒ 0 + aviso
        assert_eq!(rot(r#""side_data_list":[{"rotation":1e300}]"#), 0);
    }

    #[test]
    fn stream_flood_is_rejected() {
        let many: Vec<String> = (0..=MAX_STREAMS)
            .map(|i| format!(r#"{{"index":{i},"codec_type":"data"}}"#))
            .collect();
        let e = normalize_ffprobe_json(&doc(&many.join(","), MP4)).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaUnsupportedFormat);
    }
}
