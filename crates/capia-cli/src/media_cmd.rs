//! `capia job …`, `capia cache …`, `capia media index|frame|waveform|proxy` e
//! `capia asset force-relink|relink-folder|import --async`: traduzem argumentos para a fachada
//! `capia-project`. Nenhuma regra de mídia mora aqui.

use crate::assets_cmd::{locate, project_err, usage};
use crate::cli::{Args, Io, actor};
use capia_assets::ScanOptions;
use capia_media::{FfprobeBackend, MediaToolchain, ProxyAudio, ProxyProfileV1};
use capia_model::AssetId;
use capia_project::{
    JobHandle, JobId, JobSnapshot, JobState, PipelineOptions, Priority, Project, ProjectError,
    PumpEvent,
};
use capia_store::StoreOptions;
use capia_time::Ticks;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn open(path: &str, opts: &StoreOptions, io: &mut Io<'_>, json: bool) -> Result<Project, i32> {
    Project::open(Path::new(path), opts).map_err(|e| project_err(io, &ProjectError::Store(e), json))
}

fn parse_u32(
    v: &Option<String>,
    name: &str,
    min: u32,
    max: u32,
    default: u32,
) -> Result<u32, String> {
    match v.as_deref() {
        None => Ok(default),
        Some(s) => s
            .parse::<u32>()
            .ok()
            .filter(|n| (min..=max).contains(n))
            .ok_or_else(|| format!("{name} must be {min}..={max} (got {s})")),
    }
}

fn priority(a: &Args) -> Result<Priority, String> {
    match a.priority.as_deref() {
        None | Some("normal") => Ok(Priority::Normal),
        Some("interactive") => Ok(Priority::Interactive),
        Some("background") => Ok(Priority::Background),
        Some(o) => Err(format!(
            "--priority must be interactive, normal or background (got {o})"
        )),
    }
}

fn print_job(io: &mut Io<'_>, j: &JobSnapshot) {
    let pct = match j.progress.done.min(j.progress.total).checked_mul(100) {
        Some(n) if j.progress.total > 0 => format!("{:>3}%", n / j.progress.total),
        _ => "  — ".into(),
    };
    io.print(&format!(
        "{}  {:<11} {}  {:<10} {}{}",
        j.id,
        j.state.as_str(),
        pct,
        j.kind.as_str(),
        j.label,
        j.error
            .as_ref()
            .map_or(String::new(), |e| format!("  [{}]", e.code))
    ));
}

/// Inicia o executor e devolve o projeto pronto para submeter jobs.
fn start(
    project: &mut Project,
    tc: MediaToolchain,
    io: &mut Io<'_>,
    json: bool,
) -> Result<(), i32> {
    project
        .start_pipeline(PipelineOptions::new(tc))
        .map(|_| ())
        .map_err(|e| project_err(io, &e, json))
}

/// Espera o job terminar (pump a cada 100 ms); `--progress` imprime o andamento no stderr.
fn wait(project: &mut Project, handle: &JobHandle, args: &Args, io: &mut Io<'_>) -> JobSnapshot {
    let who = actor(args).unwrap_or_else(|_| capia_commands::Actor::system());
    let mut last = (0, 0);
    loop {
        let snap = handle.wait_timeout(Duration::from_millis(100));
        let _ = project.pump(&who);
        let cur = handle.snapshot();
        if args.progress && (cur.progress.done, cur.progress.total) != last {
            last = (cur.progress.done, cur.progress.total);
            let _ = writeln!(io.err, "progress: {}/{}", last.0, last.1);
        }
        if let Some(s) = snap {
            return s;
        }
    }
}

fn job_failure(io: &mut Io<'_>, snap: &JobSnapshot, json: bool) -> i32 {
    let e = snap
        .error
        .clone()
        .unwrap_or_else(|| capia_jobs_error(snap.state));
    io.fail(&e, &format!("{}: {}", e.code, e.message), json)
}

fn capia_jobs_error(state: JobState) -> capia_project::JobError {
    capia_project::JobError::new(
        "JOB_NOT_COMPLETED",
        format!("the job ended {}", state.as_str()),
    )
}

pub(crate) fn run(cmd: &str, args: &Args, opts: &StoreOptions, io: &mut Io<'_>) -> i32 {
    let json = args.json;
    let Some(sub) = args.positional.first().map(String::as_str) else {
        return usage(io, &format!("`{cmd}` needs a subcommand"));
    };
    let rest = &args.positional[1..];
    let Some(path) = rest.first() else {
        return usage(io, "missing project path");
    };
    let project = match open(path, opts, io, json) {
        Ok(p) => p,
        Err(c) => return c,
    };
    match (cmd, sub) {
        ("job", "list") => {
            let state = match args.state.as_deref() {
                None => None,
                Some(s) => match JobState::parse(s) {
                    Some(st) => Some(st),
                    None => {
                        return usage(
                            io,
                            "--state must be queued|running|completed|failed|cancelled|interrupted",
                        );
                    }
                },
            };
            // sem dono vivo, o que ficou running/queued de um processo morto vira `interrupted`
            if let Err(e) = project.recover_jobs() {
                return project_err(io, &e, json);
            }
            match project.jobs(state, 200) {
                Ok(jobs) if json => {
                    io.print_json(&jobs, args.pretty);
                    0
                }
                Ok(jobs) => {
                    for j in &jobs {
                        print_job(io, j);
                    }
                    io.print(&format!("{} job(s)", jobs.len()));
                    0
                }
                Err(e) => project_err(io, &e, json),
            }
        }
        ("job", "status") | ("job", "cancel") => {
            let Some(id) = rest.get(1) else {
                return usage(io, "missing job id");
            };
            let id = JobId(id.clone());
            if sub == "status" {
                if let Err(e) = project.recover_jobs() {
                    return project_err(io, &e, json);
                }
                return match project.job(&id) {
                    Ok(Some(j)) if json => {
                        io.print_json(&j, args.pretty);
                        0
                    }
                    Ok(Some(j)) => {
                        print_job(io, &j);
                        0
                    }
                    Ok(None) => project_err(
                        io,
                        &ProjectError::Invalid {
                            code: "JOB_NOT_FOUND",
                            message: format!("job {id} does not exist"),
                            details: None,
                        },
                        json,
                    ),
                    Err(e) => project_err(io, &e, json),
                };
            }
            match project.cancel_job(&id) {
                Ok(true) => {
                    if json {
                        io.print_json(&json!({ "job_id": id.0, "cancel_requested": true }), false);
                    } else {
                        io.print(&format!("cancel requested for {id}"));
                    }
                    0
                }
                Ok(false) => project_err(
                    io,
                    &ProjectError::Invalid {
                        code: "JOB_NOT_CANCELLABLE",
                        message: format!("job {id} does not exist or already finished"),
                        details: None,
                    },
                    json,
                ),
                Err(e) => project_err(io, &e, json),
            }
        }
        ("cache", "info") => match project.cache_usage() {
            Ok(u) if json => {
                io.print_json(&u, args.pretty);
                0
            }
            Ok(u) => {
                io.print(&format!("{} file(s), {} bytes", u.files, u.bytes));
                for (op, (n, b)) in &u.by_op {
                    io.print(&format!("  {op}: {n} file(s), {b} bytes"));
                }
                if u.temp_files > 0 {
                    io.print(&format!(
                        "  leftover temp: {} file(s), {} bytes",
                        u.temp_files, u.temp_bytes
                    ));
                }
                0
            }
            Err(e) => project_err(io, &e, json),
        },
        ("cache", "clean") => {
            match project.cache_clean(args.all) {
                Ok(r) if json => {
                    io.print_json(&json!({ "removed_files": r.removed_files, "removed_bytes": r.removed_bytes }), false);
                    0
                }
                Ok(r) => {
                    io.print(&format!(
                        "removed {} file(s), {} bytes",
                        r.removed_files, r.removed_bytes
                    ));
                    0
                }
                Err(e) => project_err(io, &e, json),
            }
        }
        _ => usage(io, &format!("unknown {cmd} subcommand `{sub}`")),
    }
}

pub(crate) fn run_media(
    sub: &str,
    rest: &[String],
    args: &Args,
    opts: &StoreOptions,
    io: &mut Io<'_>,
) -> i32 {
    let json = args.json;
    if rest.len() != 2 {
        return usage(
            io,
            &format!("usage: capia media {sub} <projeto.capia> <asset-id>"),
        );
    }
    let id = AssetId::new(rest[1].as_str());
    let tc = match locate(args, io) {
        Ok(t) => t,
        Err(c) => return c,
    };
    let mut project = match open(&rest[0], opts, io, json) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let prio = match priority(args) {
        Ok(p) => p,
        Err(m) => return usage(io, &m),
    };
    match sub {
        "frame" => {
            let src = match project.frame_source(&id, &tc, &|| false) {
                Ok(s) => s,
                Err(e) => return project_err(io, &e, json),
            };
            let frame = if let Some(i) = &args.index {
                let Ok(i) = i.parse::<usize>() else {
                    return usage(io, "--index must be a non-negative integer");
                };
                src.frame_by_index(i, &|| false)
            } else {
                let at = match args.at.as_deref() {
                    None => Ticks::ZERO,
                    Some(s) => match capia_media::parse_decimal_ticks(s) {
                        Some(t) => t,
                        None => return usage(io, "--at must be non-negative seconds (e.g. 1.5)"),
                    },
                };
                src.frame_at(at, &|| false)
            };
            let f = match frame {
                Ok(f) => f,
                Err(e) => return project_err(io, &ProjectError::Asset(e), json),
            };
            if let Some(out) = &args.out {
                // PPM P6 (RGB): formato trivial, legível por qualquer visualizador
                let mut data = format!("P6\n{} {}\n255\n", f.width, f.height).into_bytes();
                for px in f.bytes.chunks_exact(4) {
                    data.extend_from_slice(&px[..3]);
                }
                if let Err(e) = std::fs::write(out, data) {
                    return project_err(
                        io,
                        &ProjectError::Asset(capia_assets::AssetError::new(
                            capia_assets::AssetErrorCode::AssetIo,
                            format!("cannot write `{out}`: {e}"),
                        )),
                        json,
                    );
                }
            }
            let v = json!({
                "asset_id": id.as_str(),
                "index": f.index,
                "pts": f.pts,
                "time_ticks": f.time.0,
                "width": f.width,
                "height": f.height,
                "stride": f.stride,
                "pixel_format": "rgba8",
                "sha256": capia_assets::hash_reader(&f.bytes[..]).map(|d| d.hash.hex().to_owned()).unwrap_or_default(),
                "out": args.out,
            });
            if json {
                io.print_json(&v, args.pretty);
            } else {
                io.print(&format!(
                    "frame #{} pts {} ({}x{}) sha256 {}",
                    f.index,
                    f.pts,
                    f.width,
                    f.height,
                    v["sha256"].as_str().unwrap_or("")
                ));
            }
            0
        }
        "index" | "waveform" | "proxy" => {
            if let Err(c) = start(&mut project, tc.clone(), io, json) {
                return c;
            }
            let submitted = match sub {
                "index" => project.submit_frame_index(&id, prio),
                "waveform" => project.submit_waveform(&id, prio),
                _ => {
                    let q = match parse_u32(&args.quality, "--quality", 2, 31, 5) {
                        Ok(v) => v,
                        Err(m) => return usage(io, &m),
                    };
                    let w = match parse_u32(&args.max_width, "--max-width", 16, 8192, 960) {
                        Ok(v) => v,
                        Err(m) => return usage(io, &m),
                    };
                    let h = match parse_u32(&args.max_height, "--max-height", 16, 8192, 540) {
                        Ok(v) => v,
                        Err(m) => return usage(io, &m),
                    };
                    project.submit_proxy(
                        &id,
                        ProxyProfileV1 {
                            max_width: w,
                            max_height: h,
                            jpeg_quality: u8::try_from(q).unwrap_or(5),
                            audio: if args.no_audio {
                                ProxyAudio::None
                            } else {
                                ProxyAudio::Aac(128)
                            },
                            ..ProxyProfileV1::default()
                        },
                        prio,
                    )
                }
            };
            let submitted = match submitted {
                Ok(s) => s,
                Err(e) => return project_err(io, &e, json),
            };
            let job_id = submitted.handle.id();
            if args.progress {
                let _ = writeln!(io.err, "job: {job_id}");
            }
            let snap = wait(&mut project, &submitted.handle, args, io);
            if snap.state != JobState::Completed {
                return job_failure(io, &snap, json);
            }
            let result = snap.result.clone().unwrap_or(Value::Null);
            let mut out = json!({ "job_id": job_id.0, "result": result });
            match sub {
                "index" => {
                    if let Ok(src) = project.frame_source(&id, &tc, &|| false) {
                        let ix = src.index();
                        out["frames"] = json!(ix.len());
                        out["keyframes"] = json!(ix.keyframe_count());
                        out["duration_ticks"] = json!(ix.duration().map(|d| d.0));
                        out["time_base"] = json!(ix.time_base().to_string());
                    }
                }
                "waveform" => {
                    if let Ok(w) = project.waveform(&id, &tc, &|| false) {
                        out["total_samples"] = json!(w.total_samples());
                        out["sample_rate"] = json!(w.sample_rate());
                        out["levels"] = json!(w.level_count());
                        if let Some(b) = &args.buckets {
                            let n = b.parse::<usize>().unwrap_or(0).clamp(1, 4096);
                            let peaks: Vec<Value> = w
                                .query(Ticks::ZERO, w.duration(), n)
                                .iter()
                                .map(|p| json!([p.min, p.max, p.rms]))
                                .collect();
                            out["peaks"] = json!(peaks);
                        }
                    }
                }
                _ => {}
            }
            if json {
                io.print_json(&out, args.pretty);
            } else {
                io.print(&format!(
                    "{} ok: {}",
                    sub,
                    out["result"]["path"].as_str().unwrap_or("")
                ));
            }
            0
        }
        _ => usage(io, "unknown media subcommand"),
    }
}

pub(crate) fn run_asset_ext(
    sub: &str,
    rest: &[String],
    args: &Args,
    opts: &StoreOptions,
    io: &mut Io<'_>,
) -> i32 {
    let json = args.json;
    match sub {
        "force-relink" => {
            if rest.len() != 3 {
                return usage(
                    io,
                    "usage: capia asset force-relink <projeto.capia> <asset-id> <arquivo> [--dry-run]",
                );
            }
            let tc = match locate(args, io) {
                Ok(t) => t,
                Err(c) => return c,
            };
            let who = match actor(args) {
                Ok(a) => a,
                Err(m) => return usage(io, &m),
            };
            let mut project = match open(&rest[0], opts, io, json) {
                Ok(p) => p,
                Err(c) => return c,
            };
            let id = AssetId::new(rest[1].as_str());
            match project.force_relink_asset(
                &who,
                &id,
                Path::new(&rest[2]),
                &FfprobeBackend::new(tc),
                args.dry_run,
            ) {
                Ok(r) if json => {
                    io.print_json(&r, args.pretty);
                    0
                }
                Ok(r) => {
                    io.print(&format!(
                        "{}force-relinked {}: {} → {} ({} dependent clip(s))",
                        if r.dry_run { "[dry-run] " } else { "" },
                        r.asset_id,
                        r.from,
                        r.to,
                        r.dependents.len()
                    ));
                    0
                }
                Err(e) => project_err(io, &e, json),
            }
        }
        "relink-folder" => {
            if rest.len() != 2 {
                return usage(
                    io,
                    "usage: capia asset relink-folder <projeto.capia> <pasta>",
                );
            }
            let depth = match parse_u32(
                &args.max_depth,
                "--max-depth",
                0,
                64,
                ScanOptions::default().max_depth,
            ) {
                Ok(v) => v,
                Err(m) => return usage(io, &m),
            };
            let files = match parse_u32(&args.max_files, "--max-files", 1, 5_000_000, 500_000) {
                Ok(v) => v,
                Err(m) => return usage(io, &m),
            };
            let scan = ScanOptions {
                max_depth: depth,
                max_files: u64::from(files),
                follow_links: args.follow_links,
            };
            let mut project = match open(&rest[0], opts, io, json) {
                Ok(p) => p,
                Err(c) => return c,
            };
            match project.batch_relink_folder(Path::new(&rest[1]), &scan, None, &|| false) {
                Ok((report, applied, errors)) if json => {
                    io.print_json(
                        &json!({ "report": report, "applied": applied, "apply_errors": errors }),
                        args.pretty,
                    );
                    0
                }
                Ok((report, applied, errors)) => {
                    io.print(&format!(
                        "relinked {} · unresolved {} · ambiguous {} · rejected {} · errors {} (scanned {} files, hashed {})",
                        applied.len(),
                        report.unresolved.len(),
                        report.ambiguous.len(),
                        report.rejected.len(),
                        report.errors.len() + errors.len(),
                        report.scanned_files,
                        report.hashed_files
                    ));
                    0
                }
                Err(e) => project_err(io, &e, json),
            }
        }
        _ => usage(io, "unknown asset subcommand"),
    }
}

pub(crate) fn import_async(
    rest: &[String],
    args: &Args,
    opts: &StoreOptions,
    io: &mut Io<'_>,
) -> i32 {
    let json = args.json;
    if rest.len() != 2 {
        return usage(
            io,
            "usage: capia asset import <projeto.capia> <arquivo> --async",
        );
    }
    let tc = match locate(args, io) {
        Ok(t) => t,
        Err(c) => return c,
    };
    let who = match actor(args) {
        Ok(a) => a,
        Err(m) => return usage(io, &m),
    };
    let mut project = match open(&rest[0], opts, io, json) {
        Ok(p) => p,
        Err(c) => return c,
    };
    if let Err(c) = start(&mut project, tc, io, json) {
        return c;
    }
    let ticket = match project.import_asset_async(&PathBuf::from(&rest[1])) {
        Ok(t) => t,
        Err(e) => return project_err(io, &e, json),
    };
    // o ticket sai imediatamente (antes do hash completo); depois aguardamos a finalização
    if json {
        io.print_json(&json!({ "ticket": ticket }), false);
    } else {
        io.print(&format!(
            "ticket {} pending ({} bytes)",
            ticket.ticket_id, ticket.size_bytes
        ));
    }
    let _ = io.out.flush();
    loop {
        match project.pump(&who) {
            Ok(events) => {
                if let Some(ev) = events.into_iter().next() {
                    return match ev {
                        PumpEvent::ImportFinalized { result, .. } => {
                            if json {
                                io.print_json(&json!({ "finalized": result }), args.pretty);
                            } else {
                                io.print(&format!(
                                    "finalized {} ({})",
                                    result.asset_id, result.record.display_name
                                ));
                            }
                            0
                        }
                        PumpEvent::ImportFailed { error, .. } => {
                            io.fail(&error, &format!("{}: {}", error.code, error.message), json)
                        }
                        other => {
                            let e = json!({ "code": "IMPORT_NOT_FINALIZED", "message": format!("{other:?}") });
                            io.fail(&e, "import was not finalized", json)
                        }
                    };
                }
            }
            Err(e) => return project_err(io, &e, json),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
