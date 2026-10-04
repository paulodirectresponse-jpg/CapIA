//! `capia render …`, `capia export …` e `capia media encoders`: traduzem argumentos para a fachada
//! `capia-project` (render/export headless). Nenhuma regra de render/export mora aqui.

use crate::assets_cmd::{locate, project_err, usage};
use crate::cli::{Args, Io};
use capia_media::{ExportCodec, detect_encoders};
use capia_project::{
    ExportOptions, Image, Mp4Options, Project, ProjectError, RenderServices, RenderSettings,
    frame_digest, write_wav_f32,
};
use capia_store::StoreOptions;
use capia_time::{Ticks, TimeRange};
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

fn parse_u32(v: &Option<String>, name: &str, min: u32, max: u32, d: u32) -> Result<u32, String> {
    match v.as_deref() {
        None => Ok(d),
        Some(s) => s
            .parse::<u32>()
            .ok()
            .filter(|n| (min..=max).contains(n))
            .ok_or_else(|| format!("{name} must be {min}..={max} (got {s})")),
    }
}

fn seconds(v: &Option<String>, name: &str) -> Result<Option<Ticks>, String> {
    match v.as_deref() {
        None => Ok(None),
        Some(s) => capia_media::parse_decimal_ticks(s)
            .map(Some)
            .ok_or_else(|| {
                format!("{name} must be a non-negative decimal number of seconds (got {s})")
            }),
    }
}

/// `capia media encoders`
pub(crate) fn encoders(args: &Args, io: &mut Io<'_>) -> i32 {
    let tc = match locate(args, io) {
        Ok(t) => t,
        Err(c) => return c,
    };
    match detect_encoders(&tc) {
        Ok(caps) if args.json => {
            io.print_json(
                &json!({ "ffmpeg": tc.version, "encoders": caps }),
                args.pretty,
            );
            0
        }
        Ok(caps) => {
            io.print(&format!("ffmpeg: {}", tc.version));
            for c in &caps {
                let state = if c.available {
                    "available".to_owned()
                } else {
                    format!(
                        "unavailable — {}",
                        c.reason_unavailable.as_deref().unwrap_or("?")
                    )
                };
                io.print(&format!(
                    "{:<18} {:<6} {:<8} {:<22} {}",
                    c.ffmpeg_name,
                    c.codec,
                    if c.hardware { "hardware" } else { "software" },
                    format!("{:?}", c.policy),
                    state
                ));
            }
            0
        }
        Err(e) => project_err(io, &ProjectError::Asset(e.into()), args.json),
    }
}

struct Common {
    project: Project,
    services: Arc<RenderServices>,
    seq: capia_model::SequenceId,
    settings: RenderSettings,
    range: TimeRange,
}

fn common(args: &Args, path: &str, opts: &StoreOptions, io: &mut Io<'_>) -> Result<Common, i32> {
    let json = args.json;
    let Some(seq) = args.sequence.clone() else {
        return Err(usage(io, "--sequence is required"));
    };
    let wh = (|| -> Result<(u32, u32, Option<Ticks>, Option<Ticks>), String> {
        Ok((
            parse_u32(&args.width, "--width", 1, 16384, 1920)?,
            parse_u32(&args.height, "--height", 1, 16384, 1080)?,
            seconds(&args.start, "--start")?,
            seconds(&args.duration, "--duration")?,
        ))
    })();
    let (w, h, start, dur) = match wh {
        Ok(v) => v,
        Err(m) => return Err(usage(io, &m)),
    };
    let tc = locate(args, io)?;
    let project = Project::open(Path::new(path), opts)
        .map_err(|e| project_err(io, &ProjectError::Store(e), json))?;
    let seq: capia_model::SequenceId = seq.as_str().into();
    let graph = project
        .render_graph(&seq)
        .map_err(|e| project_err(io, &e, json))?;
    let gs = graph.sequence(&seq).map_err(|e| {
        project_err(
            io,
            &ProjectError::Invalid {
                code: e.code,
                message: e.message,
                details: None,
            },
            json,
        )
    })?;
    let fd = gs.frame_rate.frame_duration();
    let start_t = start.unwrap_or(Ticks(0));
    let dur_t = match (dur, args.frames.as_deref()) {
        (Some(_), Some(_)) => return Err(usage(io, "use --duration or --frames, not both")),
        (Some(d), None) => d,
        (None, Some(n)) => match n
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .and_then(|n| n.checked_mul(fd.0))
        {
            Some(t) => Ticks(t),
            None => return Err(usage(io, "--frames must be a positive integer")),
        },
        (None, None) => Ticks((gs.duration.0 - start_t.0).max(0)),
    };
    let mut settings = RenderSettings::new(w, h);
    if let Ok(r) = parse_u32(&args.rate, "--rate", 8000, 192_000, 48_000) {
        settings.audio_sample_rate = r;
    }
    if let Ok(c) = parse_u32(&args.channels, "--channels", 1, 8, 2) {
        settings.audio_channels = c;
    }
    Ok(Common {
        project,
        services: Arc::new(RenderServices::new(tc)),
        seq,
        settings,
        range: TimeRange::new(start_t, dur_t),
    })
}

fn write_ppm(path: &Path, img: &Image) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(f, "P6\n{} {}\n255\n", img.width, img.height)?;
    for px in img.data.chunks_exact(4) {
        f.write_all(&px[..3])?;
    }
    f.flush()
}

pub(crate) fn run(cmd: &str, args: &Args, opts: &StoreOptions, io: &mut Io<'_>) -> i32 {
    let json = args.json;
    let (Some(sub), Some(path)) = (args.positional.first(), args.positional.get(1)) else {
        return usage(io, &format!("usage: capia {cmd} <sub> <projeto.capia> …"));
    };
    let c = match common(args, path, opts, io) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let never = || false;
    match (cmd, sub.as_str()) {
        ("render", "frame") => {
            let rate = c
                .project
                .render_graph(&c.seq)
                .ok()
                .and_then(|g| g.sequence(&c.seq).ok().map(|s| s.frame_rate));
            let at = match (args.at.as_deref(), args.frame.as_deref(), rate) {
                (Some(_), Some(_), _) => return usage(io, "use --at or --frame, not both"),
                (Some(s), None, _) => match capia_media::parse_decimal_ticks(s) {
                    Some(t) => t,
                    None => return usage(io, "--at must be a decimal number of seconds"),
                },
                (None, Some(n), Some(r)) => {
                    match n
                        .parse::<i64>()
                        .ok()
                        .and_then(|n| r.frames_to_ticks(n).ok())
                    {
                        Some(t) => t,
                        None => return usage(io, "--frame must be a non-negative integer"),
                    }
                }
                _ => Ticks(0),
            };
            match c.project.render_frame(&c.services, &c.seq, at, &c.settings) {
                Ok(f) => {
                    if let Some(out) = &args.out
                        && let Err(e) = write_ppm(Path::new(out), &f.image)
                    {
                        return project_err(
                            io,
                            &ProjectError::Invalid {
                                code: "RENDER_OUTPUT",
                                message: e.to_string(),
                                details: None,
                            },
                            json,
                        );
                    }
                    let warnings: Vec<_> = f.warnings.iter().map(|w| w.code).collect();
                    let v = json!({
                        "sequence": c.seq.as_str(),
                        "time_ticks": at.0,
                        "width": f.image.width,
                        "height": f.image.height,
                        "digest": frame_digest(&f.image),
                        "warnings": warnings,
                    });
                    if json {
                        io.print_json(&v, args.pretty);
                    } else {
                        io.print(&format!(
                            "frame {}x{} digest {}",
                            f.image.width,
                            f.image.height,
                            frame_digest(&f.image)
                        ));
                    }
                    0
                }
                Err(e) => project_err(io, &e, json),
            }
        }
        ("render", "audio") => {
            let Some(out) = &args.out else {
                return usage(io, "--out audio.wav is required");
            };
            match c
                .project
                .render_audio_range(&c.services, &c.seq, c.range, &c.settings)
            {
                Ok((buf, warnings)) => {
                    if let Err(e) = write_wav_f32(Path::new(out), &buf) {
                        return project_err(io, &e, json);
                    }
                    let peak = buf.samples.iter().fold(0f32, |m, s| m.max(s.abs()));
                    let v = json!({
                        "frames": buf.samples.len() / buf.channels.max(1) as usize,
                        "sample_rate": buf.sample_rate,
                        "channels": buf.channels,
                        "peak": peak,
                        "warnings": warnings.iter().map(|w| w.code).collect::<Vec<_>>(),
                    });
                    if json {
                        io.print_json(&v, args.pretty);
                    } else {
                        io.print(&format!("wrote {out}: {v}"));
                    }
                    0
                }
                Err(e) => project_err(io, &e, json),
            }
        }
        ("export", "intermediate") => {
            let Some(out) = &args.out else {
                return usage(io, "--out PASTA is required");
            };
            match c.project.export_intermediate(
                &c.services,
                &c.seq,
                c.range,
                &c.settings,
                Path::new(out),
                &ExportOptions {
                    overwrite: args.overwrite,
                },
                &never,
            ) {
                Ok(r) if json => {
                    io.print_json(&r, args.pretty);
                    0
                }
                Ok(r) => {
                    io.print(&format!(
                        "exported {} frames ({}x{}) + {} audio frames to {}",
                        r.frames,
                        r.width,
                        r.height,
                        r.audio_frames,
                        r.path.display()
                    ));
                    0
                }
                Err(e) => project_err(io, &e, json),
            }
        }
        ("export", "mp4") => {
            let Some(out) = &args.out else {
                return usage(io, "--out arquivo.mp4 is required");
            };
            let codec = match args.codec.as_deref() {
                None | Some("h264") => ExportCodec::H264,
                Some("mpeg4-reference") => ExportCodec::Mpeg4Reference,
                Some(o) => {
                    return usage(
                        io,
                        &format!("--codec must be h264 or mpeg4-reference (got {o})"),
                    );
                }
            };
            let o = Mp4Options {
                overwrite: args.overwrite,
                codec,
                encoder: args.encoder.clone(),
                ..Mp4Options::default()
            };
            match c.project.export_mp4(
                &c.services,
                &c.seq,
                c.range,
                &c.settings,
                Path::new(out),
                &o,
                &never,
            ) {
                Ok(r) if json => {
                    io.print_json(&r, args.pretty);
                    0
                }
                Ok(r) => {
                    io.print(&format!(
                        "exported {} frames {}x{} with {} ({}) to {}",
                        r.frames,
                        r.width,
                        r.height,
                        r.encoder,
                        r.codec,
                        r.path.display()
                    ));
                    0
                }
                Err(e) => project_err(io, &e, json),
            }
        }
        _ => usage(io, &format!("unknown subcommand `{cmd} {sub}`")),
    }
}
