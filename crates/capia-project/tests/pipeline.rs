//! Pipeline de mídia ponta a ponta com FFmpeg real: import não bloqueante, cancelamento de hash,
//! arquivo mudando durante o hash, índice/waveform/proxy/decode via cache, relink em lote,
//! interrupção e retomada.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_assets::{AssetErrorCode, Availability, ScanOptions, hash_file};
use capia_commands::Actor;
use capia_jobs::{JobState, Priority};
use capia_media::{FfprobeBackend, MediaConfig, MediaToolchain, ProxyProfileV1};
use capia_model::AssetId;
use capia_project::{PipelineOptions, Project, PumpEvent};
use capia_store::{StoreOptions, Synchronous, TicketState};
use capia_time::{TICKS_PER_SECOND, Ticks};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static N: AtomicU64 = AtomicU64::new(0);

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "capia-pipe-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn project(&self) -> PathBuf {
        self.0.join("p.capia")
    }
    fn copy_fixture(&self, name: &str, to: &str) -> PathBuf {
        let dst = self.0.join(to);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::copy(fixture(name), &dst).unwrap();
        dst
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/media")
        .join(name)
}

fn toolchain() -> Option<MediaToolchain> {
    match MediaToolchain::locate(&MediaConfig::default()) {
        Ok(t) if t.ffmpeg.is_some() => Some(t),
        other => {
            assert!(
                std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
                "CAPIA_REQUIRE_FFMPEG is set but ffmpeg is unavailable: {other:?}"
            );
            eprintln!("SKIP (no ffmpeg)");
            None
        }
    }
}

macro_rules! need {
    () => {
        match toolchain() {
            Some(t) => t,
            None => return,
        }
    };
}

fn opts() -> StoreOptions {
    StoreOptions {
        synchronous: Synchronous::Normal,
        ..StoreOptions::default()
    }
}

fn user() -> Actor {
    Actor::user("test")
}

fn start(t: &Tmp, tc: &MediaToolchain) -> Project {
    let mut p = Project::create(&t.project(), &opts()).unwrap();
    p.start_pipeline(PipelineOptions::new(tc.clone())).unwrap();
    p
}

fn pump_until(p: &mut Project, what: &str, done: impl Fn(&[PumpEvent]) -> bool) -> Vec<PumpEvent> {
    let mut all = Vec::new();
    let t0 = Instant::now();
    loop {
        all.extend(p.pump(&user()).unwrap());
        if done(&all) {
            return all;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(60),
            "timeout waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn import_sync(p: &mut Project, tc: &MediaToolchain, file: &Path) -> AssetId {
    p.import_asset(&user(), file, &FfprobeBackend::new(tc.clone()))
        .unwrap()
        .asset_id
}

fn junk(path: &Path, mib: usize) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).unwrap();
    let block: Vec<u8> = (0..1024 * 1024).map(|i| (i % 253) as u8).collect();
    for _ in 0..mib {
        f.write_all(&block).unwrap();
    }
}

fn never() -> bool {
    false
}

// ---- import assíncrono --------------------------------------------------------------------------

#[test]
fn async_import_returns_a_pending_ticket_then_finalizes_with_the_sha256_identity() {
    let tc = need!();
    let t = Tmp::new("async");
    let mut p = start(&t, &tc);
    let file = t.copy_fixture("video_audio.mp4", "m/clip.mp4");
    let ticket = p.import_asset_async(&file).unwrap();
    assert_eq!(ticket.state, TicketState::Pending);
    assert!(ticket.fingerprint.starts_with("fp1:"));
    // o asset ainda não existe: só existe o ticket
    assert!(p.assets().unwrap().is_empty());
    assert_eq!(
        p.ticket(&ticket.ticket_id).unwrap().unwrap().state,
        TicketState::Pending
    );
    let events = pump_until(&mut p, "finalize", |e| !e.is_empty());
    let PumpEvent::ImportFinalized { result, .. } = &events[0] else {
        panic!("{events:?}");
    };
    let expected = capia_assets::asset_id_for(&hash_file(&file).unwrap().hash);
    assert_eq!(
        result.asset_id, expected,
        "identity is the full SHA-256, not the fingerprint"
    );
    let row = p.ticket(&ticket.ticket_id).unwrap().unwrap();
    assert_eq!(row.state, TicketState::Finalized);
    assert_eq!(row.asset_id.as_deref(), Some(expected.as_str()));
    assert_eq!(p.assets().unwrap().len(), 1);
    // importar o mesmo conteúdo de novo: o ticket traz a dica de duplicata e o resultado é `existing`
    let dup = p.import_asset_async(&file).unwrap();
    assert_eq!(
        dup.possible_duplicate_of.as_deref(),
        Some(expected.as_str())
    );
    let ev = pump_until(&mut p, "dup", |e| !e.is_empty());
    assert!(
        matches!(&ev[0], PumpEvent::ImportFinalized { result, .. } if result.asset_id == expected)
    );
    assert_eq!(p.assets().unwrap().len(), 1);
}

#[test]
fn async_import_returns_long_before_the_full_hash_of_a_huge_file() {
    let tc = need!();
    let t = Tmp::new("latency");
    let mut p = start(&t, &tc);
    let big = t.0.join("big.bin");
    junk(&big, 384);
    let t0 = Instant::now();
    let ticket = p.import_asset_async(&big).unwrap();
    let returned = t0.elapsed();
    let full = {
        let t1 = Instant::now();
        let _ = hash_file(&big).unwrap();
        t1.elapsed()
    };
    eprintln!("import return: {returned:?} · full SHA-256 of 384 MiB: {full:?}");
    assert!(
        returned * 4 < full || returned < Duration::from_millis(50),
        "{returned:?} vs {full:?}"
    );
    // o hash em background pode ser cancelado: nada é registrado
    assert!(p.cancel_ticket(&ticket.ticket_id).unwrap());
    let ev = pump_until(&mut p, "cancel", |e| !e.is_empty());
    assert!(matches!(
        &ev[0],
        PumpEvent::ImportCancelled { .. } | PumpEvent::ImportFailed { .. }
    ));
    assert!(p.assets().unwrap().is_empty());
    let row = p.ticket(&ticket.ticket_id).unwrap().unwrap();
    assert!(
        matches!(row.state, TicketState::Cancelled | TicketState::Failed),
        "{:?}",
        row.state
    );
}

#[test]
fn a_file_that_changes_while_it_is_hashed_never_becomes_an_asset() {
    use std::io::Write;
    let tc = need!();
    let t = Tmp::new("changing");
    let mut p = start(&t, &tc);
    let big = t.0.join("grow.bin");
    junk(&big, 512);
    let ticket = p.import_asset_async(&big).unwrap();
    // cresce enquanto o job lê
    let mut f = std::fs::OpenOptions::new().append(true).open(&big).unwrap();
    f.write_all(b"appended while hashing").unwrap();
    drop(f);
    let ev = pump_until(&mut p, "changed", |e| !e.is_empty());
    match &ev[0] {
        PumpEvent::ImportFailed { error, .. } => {
            assert!(
                error.code == AssetErrorCode::AssetChangedDuringProcessing.as_str()
                    || error.code.starts_with("MEDIA_"),
                "{error:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(p.assets().unwrap().is_empty());
    assert_eq!(
        p.ticket(&ticket.ticket_id).unwrap().unwrap().state,
        TicketState::Failed
    );
}

// ---- derivados --------------------------------------------------------------------------------

#[test]
fn index_waveform_proxy_and_decode_through_jobs_and_the_cache() {
    let tc = need!();
    let t = Tmp::new("derived");
    let mut p = start(&t, &tc);
    let video = import_sync(&mut p, &tc, &t.copy_fixture("cfr_gop.mp4", "cfr.mp4"));
    let audio = import_sync(&mut p, &tc, &t.copy_fixture("tone_44k.wav", "tone.wav"));
    let av = import_sync(&mut p, &tc, &t.copy_fixture("video_audio.mp4", "va.mp4"));
    let wait = |p: &Project, s: capia_jobs::Submitted| {
        let snap = s.handle.wait();
        assert_eq!(snap.state, JobState::Completed, "{:?}", snap.error);
        let _ = p;
        snap
    };
    let idx = wait(
        &p,
        p.submit_frame_index(&video, Priority::Interactive).unwrap(),
    );
    assert_eq!(idx.result.as_ref().unwrap()["hit"], false);
    let wf = wait(&p, p.submit_waveform(&audio, Priority::Normal).unwrap());
    assert_eq!(wf.result.as_ref().unwrap()["hit"], false);
    let px = wait(
        &p,
        p.submit_proxy(
            &av,
            ProxyProfileV1 {
                max_width: 32,
                max_height: 32,
                ..Default::default()
            },
            Priority::Background,
        )
        .unwrap(),
    );
    let proxy_path = PathBuf::from(px.result.as_ref().unwrap()["path"].as_str().unwrap());
    assert!(proxy_path.is_file());
    // segunda vez: cache hit, sem regenerar
    let again = wait(
        &p,
        p.submit_frame_index(&video, Priority::Interactive).unwrap(),
    );
    assert_eq!(again.result.as_ref().unwrap()["hit"], true);
    // o estado ficou persistido
    let jobs = p.jobs(None, 50).unwrap();
    assert!(jobs.len() >= 4 && jobs.iter().all(|j| j.state == JobState::Completed));
    // decode preciso a partir do projeto (quadro 30 do vídeo identificável)
    let src = p.frame_source(&video, &tc, &never).unwrap();
    assert_eq!(src.index().len(), 50);
    let f = src
        .frame_at(Ticks(30 * TICKS_PER_SECOND / 25), &never)
        .unwrap();
    assert_eq!(f.index, 30);
    let wave = p.waveform(&audio, &tc, &never).unwrap();
    assert_eq!(wave.total_samples(), 44_100);
    // limpar o cache não quebra nada: regenera
    let rep = p.cache_clean(true).unwrap();
    assert!(rep.removed_files >= 3);
    assert_eq!(p.cache_usage().unwrap().files, 0);
    let regen = wait(
        &p,
        p.submit_frame_index(&video, Priority::Interactive).unwrap(),
    );
    assert_eq!(regen.result.as_ref().unwrap()["hit"], false);
}

#[test]
fn identical_concurrent_submissions_are_deduplicated() {
    let tc = need!();
    let t = Tmp::new("dedup");
    let mut p = start(&t, &tc);
    let video = import_sync(&mut p, &tc, &t.copy_fixture("cfr_gop.mp4", "cfr.mp4"));
    let a = p.submit_frame_index(&video, Priority::Normal).unwrap();
    let b = p.submit_frame_index(&video, Priority::Normal).unwrap();
    if !a.handle.is_done() {
        assert!(b.deduplicated);
        assert_eq!(a.handle.id(), b.handle.id());
    }
    a.handle.wait();
}

#[test]
fn derived_jobs_refuse_a_swapped_source_file() {
    let tc = need!();
    let t = Tmp::new("swapped");
    let mut p = start(&t, &tc);
    let file = t.copy_fixture("cfr_gop.mp4", "cfr.mp4");
    let id = import_sync(&mut p, &tc, &file);
    // troca o arquivo por outro conteúdo do MESMO tamanho (só a impressão/hash pegam)
    let mut bytes = std::fs::read(&file).unwrap();
    let mid = bytes.len() / 2;
    for b in &mut bytes[mid..] {
        *b ^= 0x55;
    }
    // a impressão amostra início/fim; força diferença no fim
    let n = bytes.len();
    bytes[n - 5] ^= 0xFF;
    std::fs::write(&file, &bytes).unwrap();
    let snap = p
        .submit_frame_index(&id, Priority::Normal)
        .unwrap()
        .handle
        .wait();
    assert_eq!(snap.state, JobState::Failed);
    assert_eq!(
        snap.error.unwrap().code,
        AssetErrorCode::AssetHashMismatch.as_str()
    );
    assert_eq!(
        p.cache_usage().unwrap().files,
        0,
        "nothing cached for a mismatching source"
    );
}

// ---- relink em lote -----------------------------------------------------------------------------

#[test]
fn batch_relink_matches_by_content_never_by_name() {
    let tc = need!();
    let t = Tmp::new("batch");
    let mut p = start(&t, &tc);
    let a = t.copy_fixture("video_audio.mp4", "orig/a.mp4");
    let b = t.copy_fixture("video_only.mp4", "orig/b.mp4");
    let c = t.copy_fixture("audio.wav", "orig/c.wav");
    let d = t.copy_fixture("tone_44k.wav", "orig/d.wav");
    let ids: Vec<AssetId> = [&a, &b, &c, &d]
        .iter()
        .map(|f| import_sync(&mut p, &tc, f))
        .collect();
    // tudo offline: os originais somem
    std::fs::remove_dir_all(t.0.join("orig")).unwrap();
    for id in &ids {
        assert_eq!(
            p.asset(id).unwrap().catalog.unwrap().status,
            Availability::Offline
        );
    }
    // nova pasta: a e b com NOMES TROCADOS, c com o nome certo mas conteúdo diferente do mesmo
    // tamanho (isca), d ausente; mais uma cópia de b (ambígua)
    let new = t.0.join("new/deep");
    std::fs::create_dir_all(&new).unwrap();
    std::fs::copy(fixture("video_audio.mp4"), new.join("b.mp4")).unwrap(); // conteúdo de A com nome de B
    std::fs::copy(fixture("video_only.mp4"), new.join("a.mp4")).unwrap(); // conteúdo de B
    std::fs::copy(fixture("video_only.mp4"), new.join("a_copy.mp4")).unwrap(); // 2ª cópia de B ⇒ ambíguo
    let mut decoy = std::fs::read(fixture("audio.wav")).unwrap();
    let last = decoy.len() - 1;
    decoy[last] ^= 0xFF; // mesmo tamanho, impressão igual (região final amostrada?) → rejeitado ou filtrado
    std::fs::write(new.join("c.wav"), &decoy).unwrap();
    let (report, applied, errors) = p
        .batch_relink_folder(&t.0.join("new"), &ScanOptions::default(), None, &never)
        .unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    // A: relinkado ao arquivo certo, mesmo estando com o nome "b.mp4"
    assert!(
        report
            .matched
            .iter()
            .any(|m| m.asset_id == ids[0] && m.path.ends_with("b.mp4"))
    );
    assert_eq!(applied.len(), 1);
    assert_eq!(
        p.asset(&ids[0]).unwrap().catalog.unwrap().status,
        Availability::Online
    );
    // B: duas cópias idênticas ⇒ ambíguo, NADA aplicado
    assert!(
        report
            .ambiguous
            .iter()
            .any(|x| x.asset_id == ids[1] && x.candidates.len() == 2)
    );
    assert_eq!(
        p.asset(&ids[1]).unwrap().catalog.unwrap().status,
        Availability::Offline
    );
    // C: a isca não casa (nome igual, conteúdo diferente) ⇒ não relinkado
    assert!(!report.matched.iter().any(|m| m.asset_id == ids[2]));
    assert!(
        report.unresolved.contains(&ids[2]) || report.rejected.iter().any(|r| r.asset_id == ids[2])
    );
    assert_eq!(
        p.asset(&ids[2]).unwrap().catalog.unwrap().status,
        Availability::Offline
    );
    // D: ausente
    assert!(report.unresolved.contains(&ids[3]));
    assert!(report.hashed_files >= 2);
}

#[test]
fn batch_relink_as_a_job_applies_through_pump() {
    let tc = need!();
    let t = Tmp::new("batchjob");
    let mut p = start(&t, &tc);
    let a = t.copy_fixture("video_audio.mp4", "orig/a.mp4");
    let id = import_sync(&mut p, &tc, &a);
    std::fs::remove_dir_all(t.0.join("orig")).unwrap();
    std::fs::create_dir_all(t.0.join("lib")).unwrap();
    std::fs::copy(fixture("video_audio.mp4"), t.0.join("lib/whatever.mov")).unwrap();
    let sub = p
        .submit_batch_relink(&t.0.join("lib"), ScanOptions::default(), None)
        .unwrap();
    let ev = pump_until(&mut p, "batch relink", |e| !e.is_empty());
    let PumpEvent::BatchRelinkApplied {
        report, applied, ..
    } = &ev[0]
    else {
        panic!("{ev:?}");
    };
    assert_eq!(report.matched.len(), 1);
    assert_eq!(applied.len(), 1);
    assert_eq!(sub.handle.snapshot().state, JobState::Completed);
    assert_eq!(
        p.asset(&id).unwrap().catalog.unwrap().status,
        Availability::Online
    );
}

// ---- interrupção e retomada --------------------------------------------------------------------

#[test]
fn closing_the_project_interrupts_running_jobs_and_a_retry_completes() {
    let tc = need!();
    let t = Tmp::new("interrupt");
    let long = t.0.join("long.mkv");
    let st = std::process::Command::new(tc.ffmpeg.as_ref().unwrap())
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
    let id;
    let job_id;
    {
        let mut p = start(&t, &tc);
        id = import_sync(&mut p, &tc, &long);
        let sub = p
            .submit_proxy(
                &id,
                ProxyProfileV1 {
                    max_width: 640,
                    max_height: 360,
                    jpeg_quality: 2,
                    audio: capia_media::ProxyAudio::None,
                    ..Default::default()
                },
                Priority::Background,
            )
            .unwrap();
        job_id = sub.handle.id();
        // espera começar
        let t0 = Instant::now();
        while sub.handle.snapshot().state != JobState::Running {
            assert!(t0.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(5));
        }
        // fecha o projeto com o job rodando
    }
    let mut p = Project::open(&t.project(), &opts()).unwrap();
    let snap = p.job(&job_id).unwrap().unwrap();
    assert!(
        matches!(snap.state, JobState::Interrupted | JobState::Running),
        "{:?}",
        snap.state
    );
    let rec = p.recover_jobs().unwrap();
    let snap = p.job(&job_id).unwrap().unwrap();
    assert_eq!(
        snap.state,
        JobState::Interrupted,
        "never completed; recovery {rec:?}"
    );
    assert!(snap.error.is_none() || snap.state == JobState::Interrupted);
    // nenhum proxy parcial publicado
    assert_eq!(p.cache_usage().unwrap().files, 0);
    // repetir funciona (novo job, mesma chave)
    p.start_pipeline(PipelineOptions::new(tc.clone())).unwrap();
    let h = p
        .submit_proxy(
            &id,
            ProxyProfileV1 {
                max_width: 160,
                max_height: 90,
                audio: capia_media::ProxyAudio::None,
                ..Default::default()
            },
            Priority::Normal,
        )
        .unwrap();
    assert_eq!(h.handle.wait().state, JobState::Completed);
    assert_ne!(h.handle.id(), job_id);
}

#[test]
fn only_one_process_can_own_the_executor() {
    let tc = need!();
    let t = Tmp::new("owner");
    let _p1 = start(&t, &tc);
    let mut p2 = Project::open(&t.project(), &opts()).unwrap();
    let e = p2
        .start_pipeline(PipelineOptions::new(tc.clone()))
        .unwrap_err();
    assert_eq!(e.code(), "JOBS_OWNED_BY_ANOTHER_PROCESS");
    // sem dono ⇒ recover_jobs não mexe em jobs de quem está vivo
    assert_eq!(p2.recover_jobs().unwrap().jobs_interrupted, 0);
}

#[test]
fn a_cancel_request_from_another_connection_reaches_the_running_job() {
    let tc = need!();
    let t = Tmp::new("xcancel");
    let long = t.0.join("long.mkv");
    let st = std::process::Command::new(tc.ffmpeg.as_ref().unwrap())
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
    let mut p = start(&t, &tc);
    let id = import_sync(&mut p, &tc, &long);
    let sub = p
        .submit_proxy(
            &id,
            ProxyProfileV1 {
                max_width: 640,
                max_height: 360,
                jpeg_quality: 2,
                audio: capia_media::ProxyAudio::None,
                ..Default::default()
            },
            Priority::Normal,
        )
        .unwrap();
    while sub.handle.snapshot().state != JobState::Running {
        std::thread::sleep(Duration::from_millis(5));
    }
    // outro "processo": conexão própria só marca a flag
    let other = capia_store::JobStore::open(&t.project(), Duration::from_secs(3)).unwrap();
    assert!(other.request_cancel(&sub.handle.id()).unwrap());
    let snap = sub
        .handle
        .wait_timeout(Duration::from_secs(15))
        .expect("the watcher cancels the job");
    assert_eq!(snap.state, JobState::Cancelled);
    assert_eq!(p.cache_usage().unwrap().files, 0);
}
