//! Crash **real** durante índice/waveform/proxy/hash: um processo filho (este binário de teste) é
//! morto de fora (SIGKILL/TerminateProcess) com o job em andamento. O pai reabre e exige: projeto
//! válido, job `interrupted` (nunca `completed`), nenhum derivado parcial publicado e retry que
//! funciona.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_commands::Actor;
use capia_jobs::{JobState, Priority};
use capia_media::{FfprobeBackend, MediaConfig, MediaToolchain, ProxyAudio, ProxyProfileV1};
use capia_model::AssetId;
use capia_project::{PipelineOptions, Project};
use capia_store::{StoreOptions, Synchronous, TicketState};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

static N: AtomicU64 = AtomicU64::new(0);

fn opts() -> StoreOptions {
    StoreOptions {
        synchronous: Synchronous::Full,
        ..StoreOptions::default()
    }
}

fn user() -> Actor {
    Actor::user("crash")
}

fn toolchain() -> Option<MediaToolchain> {
    match MediaToolchain::locate(&MediaConfig::default()) {
        Ok(t) if t.ffmpeg.is_some() => Some(t),
        other => {
            assert!(
                std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
                "{other:?}"
            );
            eprintln!("SKIP (no ffmpeg)");
            None
        }
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/media")
        .join(name)
}

/// Ponto de entrada do filho. Sem a variável é um no-op.
#[test]
fn crash_child() {
    let Ok(scenario) = std::env::var("CAPIA_CRASH_CHILD") else {
        return;
    };
    let path = PathBuf::from(std::env::var("CAPIA_CRASH_PATH").unwrap());
    let asset = AssetId::new(std::env::var("CAPIA_CRASH_ASSET").unwrap_or_default());
    let tc = MediaToolchain::locate(&MediaConfig::default()).unwrap();
    let mut p = Project::open(&path, &opts()).unwrap();
    p.start_pipeline(PipelineOptions::new(tc)).unwrap();
    match scenario.as_str() {
        "index" => {
            let h = p
                .submit_frame_index(&asset, Priority::Normal)
                .unwrap()
                .handle;
            h.wait();
        }
        "waveform" => {
            let h = p.submit_waveform(&asset, Priority::Normal).unwrap().handle;
            h.wait();
        }
        "proxy" => {
            let h = p
                .submit_proxy(
                    &asset,
                    ProxyProfileV1 {
                        max_width: 640,
                        max_height: 360,
                        jpeg_quality: 2,
                        audio: ProxyAudio::None,
                        ..Default::default()
                    },
                    Priority::Normal,
                )
                .unwrap()
                .handle;
            h.wait();
        }
        "import" => {
            let t = p
                .import_asset_async(Path::new(&std::env::var("CAPIA_CRASH_FILE").unwrap()))
                .unwrap();
            println!("TICKET {}", t.ticket_id);
            // nunca chama `pump`: o ticket fica pendente até o kill
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
        other => panic!("unknown scenario {other}"),
    }
    println!("CHILD_FINISHED");
}

/// Sobe o filho, espera ele anunciar `marker` e o MATA.
fn kill_child_at(
    scenario: &str,
    project: &Path,
    asset: &str,
    file: Option<&Path>,
    failpoint: Option<&str>,
    marker: &str,
) {
    let exe = std::env::current_exe().unwrap();
    let mut c = Command::new(exe);
    c.args(["crash_child", "--exact", "--nocapture", "--test-threads=1"])
        .env("CAPIA_CRASH_CHILD", scenario)
        .env("CAPIA_CRASH_PATH", project)
        .env("CAPIA_CRASH_ASSET", asset)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(f) = file {
        c.env("CAPIA_CRASH_FILE", f);
    }
    if let Some(fp) = failpoint {
        c.env("CAPIA_FAILPOINT", fp)
            .env("CAPIA_FAILPOINT_MODE", "park");
    }
    let mut child = c.spawn().unwrap();
    let out = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let marker = marker.to_owned();
    let mut seen = false;
    while let Ok(line) = rx.recv_timeout(Duration::from_secs(60)) {
        if line.contains(&marker) {
            seen = true;
            break;
        }
    }
    assert!(seen, "child never printed `{marker}`");
    child.kill().unwrap();
    let _ = child.wait();
}

struct Setup {
    root: PathBuf,
    project: PathBuf,
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn setup(tc: &MediaToolchain, fixture_name: &str) -> (Setup, AssetId) {
    let root = std::env::temp_dir().join(format!(
        "capia-crashm-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let project = root.join("p.capia");
    let mut p = Project::create(&project, &opts()).unwrap();
    let file = root.join(fixture_name);
    std::fs::copy(fixture(fixture_name), &file).unwrap();
    let id = p
        .import_asset(&user(), &file, &FfprobeBackend::new(tc.clone()))
        .unwrap()
        .asset_id;
    (Setup { root, project }, id)
}

/// Depois do kill: válido, job interrompido (nunca completo), nada publicado, retry funciona.
fn assert_recovered(
    s: &Setup,
    asset: &AssetId,
    tc: &MediaToolchain,
    retry: impl Fn(&Project, &AssetId) -> JobState,
) {
    assert!(
        Project::validate(&s.project).ok,
        "project must be valid after the kill"
    );
    let mut p = Project::open(&s.project, &opts()).unwrap();
    let rec = p.recover_jobs().unwrap();
    assert!(
        rec.jobs_interrupted >= 1,
        "the running job must be marked interrupted: {rec:?}"
    );
    let jobs = p.jobs(None, 50).unwrap();
    assert!(!jobs.is_empty());
    assert!(
        jobs.iter().all(|j| j.state == JobState::Interrupted),
        "no job may look completed or running: {:?}",
        jobs.iter().map(|j| j.state).collect::<Vec<_>>()
    );
    assert_eq!(
        p.cache_usage().unwrap().files,
        0,
        "no partial derived file may be published"
    );
    p.start_pipeline(PipelineOptions::new(tc.clone())).unwrap();
    assert_eq!(retry(&p, asset), JobState::Completed);
    assert_eq!(
        p.cache_usage().unwrap().files,
        1,
        "the retry published exactly one valid entry"
    );
    // varre o temporário do crash (jobs antigos) sem tocar no publicado
    let _ = p.cache_clean(false).unwrap();
    assert_eq!(p.cache_usage().unwrap().files, 1);
}

#[test]
fn killing_the_process_during_the_frame_index() {
    let Some(tc) = toolchain() else { return };
    let (s, id) = setup(&tc, "cfr_gop.mp4");
    kill_child_at(
        "index",
        &s.project,
        id.as_str(),
        None,
        Some("index_running"),
        "FAILPOINT_REACHED index_running",
    );
    assert_recovered(&s, &id, &tc, |p, id| {
        p.submit_frame_index(id, Priority::Normal)
            .unwrap()
            .handle
            .wait()
            .state
    });
}

#[test]
fn killing_the_process_during_the_waveform() {
    let Some(tc) = toolchain() else { return };
    let (s, id) = setup(&tc, "tone_44k.wav");
    kill_child_at(
        "waveform",
        &s.project,
        id.as_str(),
        None,
        Some("waveform_running"),
        "FAILPOINT_REACHED waveform_running",
    );
    assert_recovered(&s, &id, &tc, |p, id| {
        p.submit_waveform(id, Priority::Normal)
            .unwrap()
            .handle
            .wait()
            .state
    });
}

#[test]
fn killing_the_process_during_the_proxy_encode() {
    let Some(tc) = toolchain() else { return };
    // entrada longa o bastante para o ffmpeg ainda estar codificando no kill
    let root = std::env::temp_dir().join(format!("capia-crashm-long-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let long = root.join("long.mkv");
    let st = Command::new(tc.ffmpeg.as_ref().unwrap())
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=30:duration=240",
        ])
        .args(["-c:v", "mpeg4", "-q:v", "10"])
        .arg(&long)
        .status()
        .unwrap();
    assert!(st.success());
    let project = root.join("p.capia");
    let mut p = Project::create(&project, &opts()).unwrap();
    let id = p
        .import_asset(&user(), &long, &FfprobeBackend::new(tc.clone()))
        .unwrap()
        .asset_id;
    drop(p);
    let s = Setup { root, project };
    kill_child_at(
        "proxy",
        &s.project,
        id.as_str(),
        None,
        Some("proxy_running"),
        "FAILPOINT_REACHED proxy_running",
    );
    // o ffmpeg órfão morre sozinho (o pipe de progresso fechou); dá um tempo antes de checar o cache
    std::thread::sleep(Duration::from_millis(500));
    assert_recovered(&s, &id, &tc, |p, id| {
        p.submit_proxy(
            id,
            ProxyProfileV1 {
                max_width: 160,
                max_height: 90,
                audio: ProxyAudio::None,
                ..Default::default()
            },
            Priority::Normal,
        )
        .unwrap()
        .handle
        .wait()
        .state
    });
}

#[test]
fn killing_the_process_while_an_async_import_is_pending() {
    let Some(tc) = toolchain() else { return };
    let (s, _id) = setup(&tc, "cfr_gop.mp4");
    let new_file = s.root.join("second.mp4");
    std::fs::copy(fixture("video_audio.mp4"), &new_file).unwrap();
    kill_child_at("import", &s.project, "", Some(&new_file), None, "TICKET ");
    assert!(Project::validate(&s.project).ok);
    let mut p = Project::open(&s.project, &opts()).unwrap();
    let rec = p.recover_jobs().unwrap();
    assert_eq!(rec.tickets_interrupted, 1, "{rec:?}");
    let tickets = p.tickets(None).unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].state, TicketState::Interrupted);
    assert_eq!(
        p.assets().unwrap().len(),
        1,
        "the unfinished import registered nothing"
    );
    // retomar finaliza
    p.start_pipeline(PipelineOptions::new(tc.clone())).unwrap();
    p.resume_ticket(&tickets[0].ticket_id).unwrap();
    let t0 = std::time::Instant::now();
    loop {
        let ev = p.pump(&user()).unwrap();
        if !ev.is_empty() {
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(p.assets().unwrap().len(), 2);
    assert_eq!(
        p.ticket(&tickets[0].ticket_id).unwrap().unwrap().state,
        TicketState::Finalized
    );
}
