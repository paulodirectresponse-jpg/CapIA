//! `capia asset …` e `capia media probe`: traduzem argumentos para a fachada `capia-project` e
//! `capia-media`. Nenhuma regra de import/dedup/relink mora aqui.

use crate::cli::{Args, Io, actor, fmt_duration};
use capia_media::{FfprobeBackend, MediaConfig, MediaProbe, MediaToolchain};
use capia_model::AssetId;
use capia_project::{AssetView, Project, ProjectError};
use capia_store::StoreOptions;
use capia_time::{TICKS_PER_SECOND, Ticks};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) fn usage(io: &mut Io<'_>, msg: &str) -> i32 {
    let _ = std::io::Write::write_fmt(&mut io.err, format_args!("error: {msg}\n"));
    2
}

pub(crate) fn project_err(io: &mut Io<'_>, e: &ProjectError, json: bool) -> i32 {
    if json {
        let _ = writeln!(io.err, "{}", json!({ "error": e.to_json() }));
    } else {
        let _ = writeln!(io.err, "error: {e}");
        let _ = writeln!(io.err, "{}", e.to_json());
    }
    1
}

fn media_config(a: &Args) -> Result<MediaConfig, String> {
    let timeout = match a.timeout_ms.as_deref() {
        None => None,
        Some(v) => Some(Duration::from_millis(
            v.parse::<u64>()
                .ok()
                .filter(|ms| (1..=3_600_000).contains(ms))
                .ok_or_else(|| format!("--timeout-ms must be 1..=3600000 (got {v})"))?,
        )),
    };
    Ok(MediaConfig {
        ffprobe: a.ffprobe.as_ref().map(PathBuf::from),
        ffmpeg: a.ffmpeg.as_ref().map(PathBuf::from),
        bundled_dir: None,
        probe_timeout: timeout,
    })
}

pub(crate) fn locate(a: &Args, io: &mut Io<'_>) -> Result<MediaToolchain, i32> {
    let cfg = match media_config(a) {
        Ok(c) => c,
        Err(m) => return Err(usage(io, &m)),
    };
    MediaToolchain::locate(&cfg).map_err(|e| {
        let err = ProjectError::Asset(e.into());
        project_err(io, &err, a.json)
    })
}

/// Segundos decimais (`"1.5"`) → ticks por aritmética inteira (sem float).
fn parse_seconds(s: &str) -> Option<Ticks> {
    capia_media::parse_decimal_ticks(s)
}

fn print_view(io: &mut Io<'_>, v: &AssetView) {
    match &v.catalog {
        Some(r) => io.print(&format!(
            "{}  {:<8}  {:<5}  {}  {}",
            v.asset_id,
            r.status.as_str(),
            r.kind.as_str(),
            r.display_name,
            v.resolved_path.as_deref().unwrap_or(&r.location.path),
        )),
        None => io.print(&format!(
            "{}  logical   —      {}  (no media file)",
            v.asset_id,
            v.document.as_ref().map_or("", |d| d.name.as_str())
        )),
    }
}

pub(crate) fn run(cmd: &str, args: &Args, opts: &StoreOptions, io: &mut Io<'_>) -> i32 {
    let json = args.json;
    let Some(sub) = args.positional.first().map(String::as_str) else {
        return usage(io, &format!("`{cmd}` needs a subcommand"));
    };
    let rest = &args.positional[1..];

    if cmd == "media" && matches!(sub, "index" | "frame" | "waveform" | "proxy") {
        return crate::media_cmd::run_media(sub, rest, args, opts, io);
    }
    if cmd == "asset" && matches!(sub, "force-relink" | "relink-folder") {
        return crate::media_cmd::run_asset_ext(sub, rest, args, opts, io);
    }
    if cmd == "asset" && sub == "import" && args.is_async {
        return crate::media_cmd::import_async(rest, args, opts, io);
    }
    if cmd == "media" && sub == "encoders" {
        return crate::render_cmd::encoders(args, io);
    }
    if cmd == "media" {
        if sub != "probe" || rest.len() != 1 {
            return usage(io, "usage: capia media probe <arquivo>");
        }
        let tc = match locate(args, io) {
            Ok(t) => t,
            Err(code) => return code,
        };
        let version = tc.version.clone();
        return match FfprobeBackend::new(tc).probe(Path::new(&rest[0])) {
            Ok(info) if json => {
                io.print_json(
                    &json!({ "backend": { "ffprobe": version }, "media": info }),
                    args.pretty,
                );
                0
            }
            Ok(info) => {
                io.print(&format!("kind:       {}", info.kind.as_str()));
                io.print(&format!("container:  {}", info.container.formats.join(",")));
                if let Some(d) = info.duration {
                    io.print(&format!("duration:   {}", fmt_duration(d.0)));
                }
                if let Some(v) = info.video() {
                    io.print(&format!(
                        "video:      #{} {} {}x{}{}",
                        v.index,
                        v.codec,
                        v.width,
                        v.height,
                        v.frame_rate
                            .map_or(String::new(), |r| format!(" @ {r} fps"))
                    ));
                }
                if let Some(a) = info.audio() {
                    io.print(&format!(
                        "audio:      #{} {} {} ch @ {} Hz",
                        a.index, a.codec, a.channels, a.sample_rate
                    ));
                }
                for w in &info.warnings {
                    io.print(&format!("warning:    {w}"));
                }
                0
            }
            Err(e) => project_err(io, &ProjectError::Asset(e.into()), json),
        };
    }

    // `capia asset <sub> <projeto> …`
    let Some(path) = rest.first().map(PathBuf::from) else {
        return usage(io, "missing project path");
    };
    let needed = match sub {
        "import" | "inspect" | "verify" | "thumbnail" => 2,
        "relink" => 3,
        "list" => 1,
        other => return usage(io, &format!("unknown asset subcommand `{other}`")),
    };
    if rest.len() != needed {
        return usage(io, &format!("wrong number of arguments for `asset {sub}`"));
    }
    let mut project = match Project::open(&path, opts) {
        Ok(p) => p,
        Err(e) => return project_err(io, &ProjectError::Store(e), json),
    };
    let id = || AssetId::new(rest[1].as_str());
    match sub {
        "import" => {
            let tc = match locate(args, io) {
                Ok(t) => t,
                Err(code) => return code,
            };
            let who = match actor(args) {
                Ok(a) => a,
                Err(m) => return usage(io, &m),
            };
            let probe = FfprobeBackend::new(tc);
            match project.import_asset(&who, Path::new(&rest[1]), &probe) {
                Ok(r) if json => {
                    io.print_json(&r, args.pretty);
                    0
                }
                Ok(r) => {
                    io.print(&format!(
                        "{} {} ({}, {})",
                        serde_json::to_value(r.outcome)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_owned))
                            .unwrap_or_default(),
                        r.asset_id,
                        r.record.kind.as_str(),
                        r.record.display_name
                    ));
                    0
                }
                Err(e) => project_err(io, &e, json),
            }
        }
        "list" => match project.assets() {
            Ok(views) if json => {
                io.print_json(&views, args.pretty);
                0
            }
            Ok(views) => {
                for v in &views {
                    print_view(io, v);
                }
                io.print(&format!("{} asset(s)", views.len()));
                0
            }
            Err(e) => project_err(io, &e, json),
        },
        "inspect" => {
            let id = id();
            match (project.asset(&id), project.asset_events(&id)) {
                (Ok(v), Ok(events)) if json => {
                    io.print_json(&json!({ "asset": v, "events": events }), args.pretty);
                    0
                }
                (Ok(v), Ok(events)) => {
                    print_view(io, &v);
                    if let Some(r) = &v.catalog {
                        io.print(&format!("  hash:  {}", r.content_hash));
                        io.print(&format!("  size:  {} bytes", r.size_bytes));
                        io.print(&format!("  path:  {}", r.location.path));
                        if let Some(d) = r.media.duration {
                            io.print(&format!(
                                "  media: {} · {}",
                                r.media.kind.as_str(),
                                fmt_duration(d.0)
                            ));
                        }
                    }
                    for e in events {
                        io.print(&format!("  event #{}: {} @ {}", e.seq, e.kind, e.at_ms));
                    }
                    0
                }
                (Err(e), _) | (_, Err(e)) => project_err(io, &e, json),
            }
        }
        "verify" => match project.verify_asset(&id()) {
            Ok(r) if json => {
                io.print_json(&r, args.pretty);
                // modified/offline também são "falha" para automação
                i32::from(r.status != capia_assets::Availability::Online)
            }
            Ok(r) => {
                io.print(&format!("{}: {}", r.asset_id, r.status.as_str()));
                i32::from(r.status != capia_assets::Availability::Online)
            }
            Err(e) => project_err(io, &e, json),
        },
        "relink" => match project.relink_asset(&id(), Path::new(&rest[2])) {
            Ok(r) if json => {
                io.print_json(&r, args.pretty);
                0
            }
            Ok(r) => {
                io.print(&format!("relinked {}: {} → {}", r.asset_id, r.from, r.to));
                0
            }
            Err(e) => project_err(io, &e, json),
        },
        "thumbnail" => {
            let tc = match locate(args, io) {
                Ok(t) => t,
                Err(code) => return code,
            };
            let at = match args.at.as_deref() {
                None => Ticks::ZERO,
                Some(s) => match parse_seconds(s) {
                    Some(t) => t,
                    None => return usage(io, "--at must be non-negative seconds (e.g. 1.5)"),
                },
            };
            let size = match args.size.as_deref() {
                None => 256,
                Some(s) => match s.parse::<u32>() {
                    Ok(v) if (16..=1024).contains(&v) => v,
                    _ => return usage(io, "--size must be 16..=1024"),
                },
            };
            match project.generate_thumbnail(&id(), at, size, &tc) {
                Ok(p) if json => {
                    io.print_json(
                        &json!({ "path": p.display().to_string(), "at_seconds": at.0 / TICKS_PER_SECOND }),
                        false,
                    );
                    0
                }
                Ok(p) => {
                    io.print(&p.display().to_string());
                    0
                }
                Err(e) => project_err(io, &e, json),
            }
        }
        _ => usage(io, "unknown asset subcommand"),
    }
}
