//! Crash **real** (processo filho morto de fora) durante: sessão de decode ativa, render de range,
//! escrita do índice de áudio, export intermediário e export MP4. Exige: `.capia` intacto, nenhum
//! derivado/saída parcial apresentado como final, limpeza do que sobrou e retry que funciona.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod lab;

use capia_jobs::{JobState, Priority};
use capia_media::{ExportCodec, MediaConfig, MediaToolchain};
use capia_model::{AssetId, TrackKind};
use capia_project::{ExportOptions, Mp4Options, PipelineOptions, Project, RenderServices};
use capia_render::RenderSettings;
use capia_time::{FrameRate, Ticks, TimeRange};
use lab::{F, Lab};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

fn no_cancel() -> bool {
    false
}

fn settings() -> RenderSettings {
    RenderSettings::new(64, 48)
}

fn range() -> TimeRange {
    TimeRange::new(Ticks(0), Ticks(45 * F))
}

/// Ponto de entrada do filho. Sem a variável é um no-op.
#[test]
fn crash_child() {
    let Ok(scenario) = std::env::var("CAPIA_CRASH_CHILD") else {
        return;
    };
    let path = PathBuf::from(std::env::var("CAPIA_CRASH_PATH").unwrap());
    let out = PathBuf::from(std::env::var("CAPIA_CRASH_OUT").unwrap_or_default());
    let tc = MediaToolchain::locate(&MediaConfig::default()).unwrap();
    let mut p = Project::open(&path, &lab::opts()).unwrap();
    let services = Arc::new(RenderServices::new(tc.clone()));
    let seq = "S".into();
    let park = |name: &str| {
        println!("CHILD_AT {name}");
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    };
    match scenario.as_str() {
        "decode" => {
            // sessão de decode ATIVA: renderiza um quadro e fica vivo com o ffmpeg do pool aberto
            p.render_frame(&services, &seq, Ticks(3 * F), &settings())
                .unwrap();
            assert_eq!(services.decode().metrics().sessions_alive, 1);
            park("decode");
        }
        "render_range" => {
            let mut n = 0;
            let _ = p.render_range(&services, &seq, range(), &settings(), &mut |_, _, _| {
                n += 1;
                if n == 5 {
                    park("render_range");
                }
                true
            });
        }
        "audio_index" => {
            let asset = AssetId::new(std::env::var("CAPIA_CRASH_ASSET").unwrap());
            p.start_pipeline(PipelineOptions::new(tc)).unwrap();
            p.submit_audio_index(&asset, Priority::Normal)
                .unwrap()
                .handle
                .wait();
        }
        "export_intermediate" => {
            let _ = p.export_intermediate(
                &services,
                &seq,
                range(),
                &settings(),
                &out,
                &ExportOptions::default(),
                &no_cancel,
            );
        }
        "export_mp4" => {
            let o = Mp4Options {
                codec: ExportCodec::Mpeg4Reference,
                ..Mp4Options::default()
            };
            let _ = p.export_mp4(&services, &seq, range(), &settings(), &out, &o, &no_cancel);
        }
        other => panic!("unknown scenario {other}"),
    }
    println!("CHILD_FINISHED");
}

fn kill_child_at(
    scenario: &str,
    project: &Path,
    asset: &str,
    out: &Path,
    failpoint: Option<&str>,
    marker: &str,
) {
    let exe = std::env::current_exe().unwrap();
    let mut c = Command::new(exe);
    c.args(["crash_child", "--exact", "--nocapture", "--test-threads=1"])
        .env("CAPIA_CRASH_CHILD", scenario)
        .env("CAPIA_CRASH_PATH", project)
        .env("CAPIA_CRASH_ASSET", asset)
        .env("CAPIA_CRASH_OUT", out)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(fp) = failpoint {
        c.env("CAPIA_FAILPOINT", fp)
            .env("CAPIA_FAILPOINT_MODE", "park");
    }
    let mut child = c.spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let mut seen = false;
    while let Ok(line) = rx.recv_timeout(Duration::from_secs(90)) {
        if line.contains(marker) {
            seen = true;
            break;
        }
    }
    assert!(seen, "child never printed `{marker}`");
    child.kill().unwrap();
    let _ = child.wait();
    // o ffmpeg órfão morre com o pipe; no Windows o handle demora a ser liberado
    std::thread::sleep(Duration::from_millis(700));
}

fn staged(lab: &Lab) -> Vec<String> {
    std::fs::read_dir(&lab.dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".partial-"))
        .collect()
}

/// Projeto com 1,5 s de vídeo+áudio na sequence S; devolve o lab (projeto FECHADO) e o asset.
fn build(tag: &str, tc: &MediaToolchain) -> (Lab, AssetId) {
    let mut lab = Lab::new(tag, tc);
    let path = lab.gen_av("a.mp4", "30", 3, 48_000);
    let a = lab.import(&path);
    lab.seq("S", FrameRate::FPS_30);
    lab.track("S", "V1", TrackKind::Visual);
    lab.track("S", "A1", TrackKind::Audio);
    lab.media("V1", "v", &a, 0, 60 * F, 0, true, false);
    lab.media("A1", "a", &a, 0, 60 * F, 0, false, true);
    lab.project = None;
    (lab, a)
}

fn fingerprint(path: &Path) -> (String, u64) {
    let i = Project::inspect(path).unwrap();
    (i.digest, i.revision)
}

fn intact(lab: &Lab, before: &(String, u64)) {
    assert!(
        Project::validate(&lab.project_path()).ok,
        "the .capia must be valid"
    );
    assert_eq!(
        &fingerprint(&lab.project_path()),
        before,
        "the document must be untouched"
    );
}

#[test]
fn killing_the_process_with_an_active_decode_session() {
    let tc = need!();
    let (lab, _a) = build("crash-decode", &tc);
    let before = fingerprint(&lab.project_path());
    kill_child_at(
        "decode",
        &lab.project_path(),
        "",
        Path::new(""),
        None,
        "CHILD_AT decode",
    );
    intact(&lab, &before);
    // reabre e renderiza de novo: funciona, sem sobra de sessão/arquivo
    let mut lab = lab;
    lab.reopen();
    let f = lab
        .p()
        .render_frame(&lab.services, &"S".into(), Ticks(3 * F), &settings())
        .unwrap();
    assert!(f.warnings.is_empty());
    assert!(staged(&lab).is_empty());
}

#[test]
fn killing_the_process_during_a_render_range() {
    let tc = need!();
    let (mut lab, _a) = build("crash-range", &tc);
    let before = fingerprint(&lab.project_path());
    kill_child_at(
        "render_range",
        &lab.project_path(),
        "",
        Path::new(""),
        None,
        "CHILD_AT render_range",
    );
    intact(&lab, &before);
    lab.reopen();
    // o cache derivado que o crash deixou é utilizável OU descartado — nunca parcial: o retry bate
    // com um render limpo de referência
    let mut a = Vec::new();
    lab.p()
        .render_range(
            &lab.services,
            &"S".into(),
            range(),
            &settings(),
            &mut |_, _, img| {
                a.push(capia_render::frame_digest(&img));
                true
            },
        )
        .unwrap();
    assert_eq!(a.len(), 45);
    let _ = lab.p().cache_clean(false).unwrap();
    lab.reopen();
    let mut b = Vec::new();
    lab.p()
        .render_range(
            &lab.services,
            &"S".into(),
            range(),
            &settings(),
            &mut |_, _, img| {
                b.push(capia_render::frame_digest(&img));
                true
            },
        )
        .unwrap();
    assert_eq!(a, b);
}

#[test]
fn killing_the_process_while_the_audio_index_is_written() {
    let tc = need!();
    let (lab, a) = build("crash-aix", &tc);
    let before = fingerprint(&lab.project_path());
    kill_child_at(
        "audio_index",
        &lab.project_path(),
        a.as_str(),
        Path::new(""),
        Some("audio_index_running"),
        "FAILPOINT_REACHED audio_index_running",
    );
    intact(&lab, &before);
    let mut p = Project::open(&lab.project_path(), &lab::opts()).unwrap();
    let rec = p.recover_jobs().unwrap();
    assert!(rec.jobs_interrupted >= 1, "{rec:?}");
    assert!(
        p.jobs(None, 50)
            .unwrap()
            .iter()
            .all(|j| j.state == JobState::Interrupted)
    );
    assert_eq!(
        p.cache_usage().unwrap().files,
        0,
        "no partial audio index may be published"
    );
    p.start_pipeline(PipelineOptions::new(tc.clone())).unwrap();
    let st = p
        .submit_audio_index(&a, Priority::Normal)
        .unwrap()
        .handle
        .wait()
        .state;
    assert_eq!(st, JobState::Completed);
    assert_eq!(p.cache_usage().unwrap().files, 1);
}

fn export_crash(scenario: &str, failpoint: &str, tag: &str, tc: &MediaToolchain) {
    let (mut lab, _a) = build(tag, tc);
    let before = fingerprint(&lab.project_path());
    let out = lab.dir.join(if scenario == "export_mp4" {
        "out.mp4"
    } else {
        "out_dir"
    });
    kill_child_at(
        scenario,
        &lab.project_path(),
        "",
        &out,
        Some(failpoint),
        &format!("FAILPOINT_REACHED {failpoint}"),
    );
    intact(&lab, &before);
    assert!(
        !out.exists(),
        "a killed export must never publish the final output ({failpoint})"
    );
    lab.reopen();
    // retry: limpa o staging do crash e publica uma saída completa e válida
    if scenario == "export_mp4" {
        let r = lab
            .p()
            .export_mp4(
                &lab.services,
                &"S".into(),
                range(),
                &settings(),
                &out,
                &Mp4Options {
                    codec: ExportCodec::Mpeg4Reference,
                    ..Mp4Options::default()
                },
                &no_cancel,
            )
            .unwrap();
        assert_eq!(r.frames, 45);
        assert!(out.is_file());
    } else {
        let r = lab
            .p()
            .export_intermediate(
                &lab.services,
                &"S".into(),
                range(),
                &settings(),
                &out,
                &ExportOptions::default(),
                &no_cancel,
            )
            .unwrap();
        assert_eq!(r.frames, 45);
        assert!(out.join("manifest.json").is_file());
    }
    assert!(
        staged(&lab).is_empty(),
        "the retry must clean the crash leftovers: {:?}",
        staged(&lab)
    );
}

#[test]
fn killing_the_process_during_an_intermediate_export() {
    let tc = need!();
    export_crash(
        "export_intermediate",
        "export_frames_running",
        "crash-inter1",
        &tc,
    );
}

#[test]
fn killing_the_process_right_before_the_intermediate_export_is_published() {
    let tc = need!();
    export_crash(
        "export_intermediate",
        "export_before_publish",
        "crash-inter2",
        &tc,
    );
}

#[test]
fn killing_the_process_during_an_mp4_export() {
    let tc = need!();
    export_crash("export_mp4", "export_frames_running", "crash-mp4a", &tc);
}

#[test]
fn killing_the_process_right_before_the_mp4_is_published() {
    let tc = need!();
    export_crash("export_mp4", "export_before_publish", "crash-mp4b", &tc);
}
