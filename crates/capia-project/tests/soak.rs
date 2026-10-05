//! Soak/leak e falha de disco (Fase 6, Track D-1). Rápidos (não ignorados) onde baratos; o soak
//! longo é `#[ignore]` e parametrizado por `CAPIA_SOAK_SECS` (driver: `tools/phase6-acceptance/
//! performance/soak.mjs`).
//!
//! * vazamento: abrir/fechar N ciclos e commit/undo N ciclos com RSS/descritores/threads LIMITADOS
//!   (lê `/proc/self`; fora do Linux o teste se declara *skip com motivo*, nunca passa em vazio);
//! * disco: diretório inexistente, somente-leitura (permissão negada; pula como root, que ignora
//!   permissões) e **disco cheio simulado** por `RLIMIT_FSIZE` (`ulimit -f` num processo filho com
//!   SIGXFSZ ignorado ⇒ `EFBIG`): o erro é estruturado e o projeto reabre íntegro.
//!
//! O que NÃO é viável aqui: ENOSPC real (exigiria tmpfs/loop montado = root + mount), falha de
//! `fsync` e disco removido no meio de um commit — ver `docs/phase6/IMPL_PERFORMANCE.md`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

use capia_commands::{Actor, Command, CommandEnvelope, Transaction};
use capia_fixtures::api::ApiProbe;
use capia_fixtures::bench::Report;
use capia_fixtures::{LargeSpec, build_large_project, proc};
use capia_project::Project;
use capia_store::{StoreErrorCode, StoreOptions, Synchronous};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use std::time::{Duration, Instant};

/// Os testes medem recursos do PROCESSO inteiro (/proc/self): rodam um por vez.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("capia-soak-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        // devolve permissões para a limpeza funcionar
        let _ = set_mode(&self.0, 0o755);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn set_mode(p: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> std::io::Result<()> {
    Ok(())
}

fn medium() -> LargeSpec {
    LargeSpec {
        sequences: 6,
        clips: 600,
        assets: 80,
        runs: 4,
        history_tail: 10,
        ..LargeSpec::small()
    }
}

fn opts() -> StoreOptions {
    StoreOptions {
        synchronous: Synchronous::Normal,
        ..StoreOptions::default()
    }
}

/// Conteúdo do documento sem o contador `revision`.
fn content(doc: &capia_model::Document) -> serde_json::Value {
    let mut v = serde_json::to_value(doc).unwrap();
    v.as_object_mut().unwrap().remove("revision");
    v
}

fn set_opacity(op: &str, clip: &str, value: f64) -> Transaction {
    Transaction {
        transaction_id: None,
        label: "soak".into(),
        base_revision: None,
        commands: vec![CommandEnvelope {
            operation_id: op.into(),
            reference: None,
            command: Command::SetProperty {
                clip: clip.into(),
                prop: "opacity".into(),
                value,
            },
        }],
        max_ops: None,
    }
}

fn first_clip(p: &Project) -> String {
    p.document()
        .sequence(&"seq_000".into())
        .unwrap()
        .clips()
        .find(|c| c.id.as_str().contains("_v1_"))
        .unwrap()
        .id
        .to_string()
}

/// `true` se o ambiente mede de verdade; senão registra o motivo (skip explícito).
fn can_measure(test: &str) -> bool {
    if proc::available() {
        true
    } else {
        eprintln!("SKIP {test}: /proc/self indisponível (só Linux mede RSS/descritores/threads)");
        false
    }
}

const MIB: u64 = 1024;

#[test]
fn open_close_cycles_do_not_leak_handles_threads_or_memory() {
    let _serial = serial();
    if !can_measure("open_close_cycles") {
        return;
    }
    let d = Dir::new("openclose");
    let path = d.0.join("p.capia");
    let st = build_large_project(&path, &medium()).unwrap();
    let cycle = || {
        let p = Project::open(&path, &opts()).unwrap();
        assert_eq!(p.document().sequences().count(), st.sequences);
        assert_eq!(p.assets().unwrap().len(), st.document_assets);
        drop(p);
    };
    for _ in 0..5 {
        cycle(); // aquecimento: caches do alocador, estatements, etc.
    }
    let (fd0, th0, rss0) = (
        proc::fd_count().unwrap(),
        proc::thread_count().unwrap(),
        proc::rss_kib().unwrap(),
    );
    for _ in 0..40 {
        cycle();
    }
    let (fd1, th1, rss1) = (
        proc::fd_count().unwrap(),
        proc::thread_count().unwrap(),
        proc::rss_kib().unwrap(),
    );
    eprintln!("open/close ×40: fd {fd0}→{fd1}, threads {th0}→{th1}, rss {rss0}→{rss1} KiB");
    assert_eq!(fd1, fd0, "file descriptors leaked");
    assert!(th1 <= th0 + 1, "threads leaked: {th0} → {th1}");
    assert!(
        rss1 <= rss0 + 24 * MIB,
        "memory grew unbounded: {rss0} → {rss1} KiB"
    );
}

#[test]
fn api_session_open_close_cycles_do_not_leak() {
    let _serial = serial();
    if !can_measure("api_session_open_close_cycles") {
        return;
    }
    let d = Dir::new("apicycle");
    let path = d.0.join("p.capia");
    build_large_project(&path, &medium()).unwrap();
    let mut api = ApiProbe::new();
    let mut cycle = || {
        api.open(&path).unwrap();
        api.call("project.snapshot", json!({})).unwrap();
        api.close().unwrap();
    };
    for _ in 0..4 {
        cycle();
    }
    let (fd0, th0, rss0) = (
        proc::fd_count().unwrap(),
        proc::thread_count().unwrap(),
        proc::rss_kib().unwrap(),
    );
    for _ in 0..25 {
        cycle();
    }
    let (fd1, th1, rss1) = (
        proc::fd_count().unwrap(),
        proc::thread_count().unwrap(),
        proc::rss_kib().unwrap(),
    );
    eprintln!("api open/close ×25: fd {fd0}→{fd1}, threads {th0}→{th1}, rss {rss0}→{rss1} KiB");
    assert_eq!(fd1, fd0, "file descriptors leaked");
    assert!(th1 <= th0 + 1, "threads leaked: {th0} → {th1}");
    assert!(rss1 <= rss0 + 24 * MIB, "memory grew: {rss0} → {rss1} KiB");
}

#[test]
fn commit_undo_cycles_keep_the_document_and_bounded_resources() {
    let _serial = serial();
    if !can_measure("commit_undo_cycles") {
        return;
    }
    let d = Dir::new("commitundo");
    let path = d.0.join("p.capia");
    build_large_project(&path, &medium()).unwrap();
    let mut p = Project::open(&path, &opts()).unwrap();
    let clip = first_clip(&p);
    let user = Actor::user("soak");
    let base = content(p.document());
    let mut n = 0u64;
    let mut cycle = |p: &mut Project| {
        n += 1;
        p.execute(&user, set_opacity(&format!("soak-{n}"), &clip, 0.33))
            .unwrap();
        p.undo(&user).unwrap();
    };
    for _ in 0..10 {
        cycle(&mut p);
    }
    let (fd0, rss0) = (proc::fd_count().unwrap(), proc::rss_kib().unwrap());
    let rev0 = p.engine().revision();
    for _ in 0..150 {
        cycle(&mut p);
    }
    let (fd1, rss1) = (proc::fd_count().unwrap(), proc::rss_kib().unwrap());
    eprintln!("commit+undo ×150: fd {fd0}→{fd1}, rss {rss0}→{rss1} KiB");
    assert_eq!(
        p.engine().revision(),
        rev0 + 300,
        "each commit and undo is a revision"
    );
    assert!(
        content(p.document()) == base,
        "commit+undo cycles must leave the content unchanged"
    );
    assert_eq!(fd1, fd0, "file descriptors leaked");
    assert!(rss1 <= rss0 + 24 * MIB, "memory grew: {rss0} → {rss1} KiB");
    // e o arquivo continua íntegro e reabre igual
    drop(p);
    assert!(Project::validate(&path).ok);
    let again = Project::open(&path, &opts()).unwrap();
    assert!(content(again.document()) == base);
}

// ---- disco ---------------------------------------------------------------------------------------

#[test]
fn creating_in_a_missing_directory_is_a_structured_error_and_leaves_nothing() {
    let _serial = serial();
    let d = Dir::new("missing");
    let path = d.0.join("no/such/dir/p.capia");
    let err = Project::create(&path, &opts()).unwrap_err();
    assert!(
        matches!(
            err.code,
            StoreErrorCode::StoreIoError | StoreErrorCode::ProjectNotFound
        ),
        "structured code expected, got {:?}: {}",
        err.code,
        err.message
    );
    assert!(!err.message.is_empty());
    assert!(!path.exists());
    // abrir o que não existe também é estruturado (não cria o arquivo)
    let ghost = d.0.join("ghost.capia");
    assert!(Project::open(&ghost, &opts()).is_err());
    assert!(!ghost.exists(), "open must never create the file");
}

#[test]
fn a_directory_in_place_of_the_project_file_is_refused_cleanly() {
    let _serial = serial();
    let d = Dir::new("isdir");
    let path = d.0.join("p.capia");
    std::fs::create_dir_all(&path).unwrap();
    assert!(Project::open(&path, &opts()).is_err());
    assert!(Project::create(&path, &opts()).is_err());
    assert!(path.is_dir(), "the directory is untouched");
}

#[test]
fn a_read_only_directory_and_file_fail_structured_and_the_project_stays_intact() {
    let _serial = serial();
    if !cfg!(unix) {
        eprintln!("SKIP read_only: permissões POSIX");
        return;
    }
    if proc::effective_uid() == Some(0) {
        eprintln!("SKIP read_only: root ignora permissões (rode como usuário comum no CI)");
        return;
    }
    let d = Dir::new("readonly");
    let path = d.0.join("p.capia");
    build_large_project(&path, &LargeSpec::small()).unwrap();
    let before = content(Project::open(&path, &opts()).unwrap().document());
    // diretório somente-leitura: criar outro projeto falha de forma estruturada
    set_mode(&d.0, 0o555).unwrap();
    let created = Project::create(&d.0.join("new.capia"), &opts());
    assert!(created.is_err(), "cannot create in a read-only dir");
    assert!(!d.0.join("new.capia").exists());
    // abrir o existente (precisa criar -wal/-shm) falha de forma estruturada, sem pânico
    let opened = Project::open(&path, &opts());
    if let Err(e) = &opened {
        assert!(!e.message.is_empty());
    }
    drop(opened);
    set_mode(&d.0, 0o755).unwrap();
    // arquivo somente-leitura: escrita falha, nada é corrompido
    set_mode(&path, 0o444).unwrap();
    if let Ok(mut p) = Project::open(&path, &opts()) {
        let clip = first_clip(&p);
        assert!(
            p.execute(&Actor::user("x"), set_opacity("ro-1", &clip, 0.1))
                .is_err()
        );
    }
    set_mode(&path, 0o644).unwrap();
    let after = content(Project::open(&path, &opts()).unwrap().document());
    assert!(
        before == after,
        "read-only failures must not change the project"
    );
    assert!(Project::validate(&path).ok);
}

const CHILD_ENV: &str = "CAPIA_SOAK_FULLDISK_PROJECT";

/// Processo filho do teste de "disco cheio": roda sob `ulimit -f` com SIGXFSZ ignorado, de modo que
/// escrever além do limite devolve `EFBIG` (como um ENOSPC). Emite uma linha `RESULT {json}`.
#[test]
#[ignore = "só como processo filho de full_disk_is_a_structured_error_and_the_project_reopens_intact"]
fn full_disk_child() {
    let Some(path) = std::env::var_os(CHILD_ENV).map(PathBuf::from) else {
        return;
    };
    let mut committed = 0u64;
    let mut last_error: Option<(String, String)> = None;
    match Project::open(&path, &opts()) {
        Err(e) => last_error = Some((format!("{:?}", e.code), e.message.clone())),
        Ok(mut p) => {
            let clip = first_clip(&p);
            for i in 0..5_000u64 {
                let v = 0.1 + (i % 9) as f64 / 10.0;
                match p.execute(
                    &Actor::user("child"),
                    set_opacity(&format!("fd-{i}"), &clip, v),
                ) {
                    Ok(_) => committed += 1,
                    Err(e) => {
                        last_error = Some((format!("{:?}", e.code), e.message.clone()));
                        break;
                    }
                }
            }
        }
    }
    println!(
        "RESULT {}",
        json!({ "committed": committed, "error": last_error.map(|(c, m)| json!({"code": c, "message": m})) })
    );
}

#[test]
fn full_disk_is_a_structured_error_and_the_project_reopens_intact() {
    let _serial = serial();
    if !cfg!(target_os = "linux") || Proc::new("sh").arg("-c").arg("true").status().is_err() {
        eprintln!("SKIP full_disk: precisa de Linux + sh (ulimit -f / RLIMIT_FSIZE)");
        return;
    }
    let d = Dir::new("fulldisk");
    let path = d.0.join("p.capia");
    build_large_project(&path, &medium()).unwrap();
    let rev_before = Project::open(&path, &opts()).unwrap().engine().revision();
    let exe = std::env::current_exe().unwrap();
    // 128 blocos de 512 B = 64 KiB: o WAL estoura depois de poucos commits. SIGXFSZ (25) ignorado
    // é herdado pelo exec ⇒ a escrita falha com EFBIG em vez de matar o processo.
    let out = Proc::new("sh")
        .arg("-c")
        .arg("trap '' 25; ulimit -f 128; exec \"$0\" --exact full_disk_child --ignored --nocapture --test-threads=1")
        .arg(&exe)
        .env(CHILD_ENV, &path)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .find_map(|l| l.find("RESULT ").map(|i| &l[i + 7..]))
        .unwrap_or_else(|| {
            panic!(
                "child produced no RESULT (killed by a signal?): status {:?}\n{text}\n{}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            )
        });
    let res: serde_json::Value = serde_json::from_str(line).unwrap();
    let committed = res["committed"].as_u64().unwrap();
    let err = &res["error"];
    assert!(
        err.is_object(),
        "the size limit never hit — the simulation is vacuous: {res}"
    );
    let msg = err["message"].as_str().unwrap();
    assert!(!err["code"].as_str().unwrap().is_empty() && !msg.is_empty());
    for raw in ["rusqlite", "panicked", "sqlite3_"] {
        assert!(!msg.contains(raw), "raw backend text leaked: {msg}");
    }
    eprintln!("full disk: {committed} commits before failing with {err}");
    // sem limite: reabre íntegro; só os commits confirmados existem (nenhum pela metade)
    let report = Project::validate(&path);
    assert!(report.ok, "{:?}", report.issues);
    let p = Project::open(&path, &opts()).unwrap();
    assert!(
        p.engine().revision() >= rev_before + committed,
        "every acknowledged commit is durable: {} < {} + {committed}",
        p.engine().revision(),
        rev_before
    );
    assert!(
        p.engine().revision() <= rev_before + committed + 1,
        "at most the failed commit may be ambiguous, and it must be whole"
    );
}

// ---- soak longo --------------------------------------------------------------------------------

/// Soak longo: `CAPIA_SOAK_SECS` (padrão 20), `CAPIA_SOAK_SPEC=default|medium` (padrão medium).
/// Ciclos de abrir/ler/fechar + commit/undo + leituras da API; amostra RSS/fd/threads; falha se
/// os recursos crescem sem limite. Grava `target/perf/phase6-soak.json`.
#[test]
#[ignore = "soak longo: tools/phase6-acceptance/performance/soak.mjs (CAPIA_SOAK_SECS)"]
fn soak_long() {
    let _serial = serial();
    if !can_measure("soak_long") {
        return;
    }
    let secs: u64 = std::env::var("CAPIA_SOAK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let spec = if std::env::var("CAPIA_SOAK_SPEC").as_deref() == Ok("default") {
        LargeSpec::default()
    } else {
        medium()
    };
    let d = Dir::new("long");
    let path = d.0.join("p.capia");
    let st = build_large_project(&path, &spec).unwrap();
    let base = {
        let p = Project::open(&path, &opts()).unwrap();
        content(p.document())
    };
    let user = Actor::user("soak");
    let mut api = ApiProbe::new();
    let t0 = Instant::now();
    let mut samples: Vec<serde_json::Value> = Vec::new();
    let (mut cycles, mut n) = (0u64, 0u64);
    let mut next_sample = Instant::now();
    while t0.elapsed() < Duration::from_secs(secs) {
        {
            let mut p = Project::open(&path, &opts()).unwrap();
            let clip = first_clip(&p);
            for _ in 0..3 {
                n += 1;
                p.execute(&user, set_opacity(&format!("sl-{n}"), &clip, 0.21))
                    .unwrap();
                p.undo(&user).unwrap();
            }
        }
        api.open(&path).unwrap();
        api.call("project.snapshot", json!({})).unwrap();
        api.call("history.list", json!({})).unwrap();
        api.close().unwrap();
        cycles += 1;
        if Instant::now() >= next_sample {
            next_sample = Instant::now() + Duration::from_secs((secs / 40).max(1));
            samples.push(json!({
                "t_s": t0.elapsed().as_secs_f64(), "cycles": cycles,
                "rss_kib": proc::rss_kib(), "fds": proc::fd_count(), "threads": proc::thread_count(),
            }));
        }
    }
    let first = samples
        .get(samples.len() / 10)
        .cloned()
        .unwrap_or(json!({}));
    let last = samples.last().cloned().unwrap_or(json!({}));
    let mut report = Report::new("phase6-soak");
    report.scalar("seconds", json!(secs));
    report.scalar("cycles", json!(cycles));
    report.scalar("fixture_clips", json!(st.clips));
    report.scalar("samples", json!(samples));
    let path_out = report.write("phase6-soak.json").unwrap();
    eprintln!(
        "soak: {cycles} cycles in {secs}s; report {}",
        path_out.display()
    );
    let (rss_a, rss_b) = (
        first["rss_kib"].as_u64().unwrap_or(0),
        last["rss_kib"].as_u64().unwrap_or(0),
    );
    assert!(
        rss_b <= rss_a + (64 * MIB).max(rss_a / 4),
        "RSS grew without bound: {rss_a} → {rss_b} KiB"
    );
    assert_eq!(first["fds"], last["fds"], "file descriptors drifted");
    assert!(last["threads"].as_u64().unwrap_or(0) <= first["threads"].as_u64().unwrap_or(0) + 1);
    let p = Project::open(&path, &opts()).unwrap();
    assert!(
        content(p.document()) == base,
        "soak cycles changed the document content"
    );
    assert!(Project::validate(&path).ok);
}
