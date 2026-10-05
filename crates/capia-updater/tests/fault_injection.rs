//! Injeção de falhas na máquina de estados do update.
//!
//! Cada efeito externamente visível (gravar estado, baixar, staging, trocar, restaurar) é um *ponto de
//! morte* duplo: o processo morre **antes** do efeito (não aconteceu) ou **depois** (aconteceu, mas o
//! processo não soube). Para **todos** os pontos de um ciclo completo o teste mata o processo, abre um
//! novo "processo" sobre o mesmo disco e exige convergência a um estado consistente.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_updater::{
    AlwaysAllow, Channel, Downloader, Ed25519Signer, Ed25519Verifier, FileStateStore, HostPolicy,
    Outcome, RecoveryReport, RollbackInfo, StateStore, SwitchGate, Switcher, UpdateError,
    UpdateManifest, UpdateRecord, UpdateState, Updater, UpdaterConfig, Version, sha256_file,
};
use capia_updater::{Artifact, MANIFEST_SCHEMA, PRODUCT};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ---- infraestrutura ----------------------------------------------------------------------------

#[derive(Debug)]
struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        let mut b = [0_u8; 8];
        getrandom::fill(&mut b).unwrap();
        let id: String = b.iter().map(|x| format!("{x:02x}")).collect();
        let p = std::env::temp_dir().join(format!("capia-upd-{tag}-{id}"));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Controla a morte do processo: ponto `2i` = antes do efeito `i`; `2i+1` = depois.
#[derive(Debug)]
struct Kill {
    counter: AtomicUsize,
    at: usize,
    fired: AtomicBool,
    dead: AtomicBool,
}

impl Kill {
    fn new(at: usize) -> Arc<Self> {
        Arc::new(Self {
            counter: AtomicUsize::new(0),
            at,
            fired: AtomicBool::new(false),
            dead: AtomicBool::new(false),
        })
    }
    fn never() -> Arc<Self> {
        Self::new(usize::MAX)
    }
    fn effects(&self) -> usize {
        self.counter.load(Ordering::SeqCst)
    }
    fn fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }
    fn new_process(&self) {
        self.dead.store(false, Ordering::SeqCst);
    }
    fn effect<T>(&self, f: impl FnOnce() -> Result<T, UpdateError>) -> Result<T, UpdateError> {
        if self.dead.load(Ordering::SeqCst) {
            return Err(UpdateError::Killed);
        }
        let i = self.counter.fetch_add(1, Ordering::SeqCst);
        if self.at == 2 * i {
            self.fired.store(true, Ordering::SeqCst);
            self.dead.store(true, Ordering::SeqCst);
            return Err(UpdateError::Killed);
        }
        let r = f();
        if self.at == 2 * i + 1 {
            self.fired.store(true, Ordering::SeqCst);
            self.dead.store(true, Ordering::SeqCst);
            return Err(UpdateError::Killed);
        }
        r
    }
}

/// "Disco" do sistema instalado (persiste entre processos).
#[derive(Debug)]
struct Sys {
    installed: Mutex<Option<Version>>,
    switches: AtomicUsize,
    restores: AtomicUsize,
    fetches: AtomicUsize,
}

impl Sys {
    fn new(v: &str) -> Arc<Self> {
        Arc::new(Self {
            installed: Mutex::new(Some(Version::parse(v).unwrap())),
            switches: AtomicUsize::new(0),
            restores: AtomicUsize::new(0),
            fetches: AtomicUsize::new(0),
        })
    }
    fn installed(&self) -> Option<Version> {
        self.installed.lock().unwrap().clone()
    }
}

#[derive(Debug)]
struct KillStore {
    inner: FileStateStore,
    kill: Arc<Kill>,
}
impl StateStore for KillStore {
    fn load(&self) -> Result<Option<UpdateRecord>, UpdateError> {
        self.inner.load()
    }
    fn save(&self, rec: &UpdateRecord) -> Result<(), UpdateError> {
        self.kill.effect(|| self.inner.save(rec))
    }
}

struct FakeSwitcher {
    sys: Arc<Sys>,
    kill: Arc<Kill>,
    /// Se `true`, o instalador "morre no meio": deixa a instalação em estado desconhecido.
    corrupt_on_switch: bool,
}

impl Switcher for FakeSwitcher {
    fn stage(&self, artifact: &Path, staging: &Path) -> Result<PathBuf, UpdateError> {
        self.kill.effect(|| {
            let dest = staging.join(artifact.file_name().unwrap());
            fs::copy(artifact, &dest)?;
            Ok(dest)
        })
    }
    fn switch(&self, staged: &Path) -> Result<(), UpdateError> {
        self.kill.effect(|| {
            self.sys.switches.fetch_add(1, Ordering::SeqCst);
            if self.corrupt_on_switch {
                *self.sys.installed.lock().unwrap() = None;
                return Err(UpdateError::Killed);
            }
            let text = fs::read_to_string(staged)?;
            let v = text
                .strip_prefix("installer:")
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .trim();
            *self.sys.installed.lock().unwrap() = Some(Version::parse(v).unwrap());
            Ok(())
        })
    }
    fn restore_previous(&self, info: &RollbackInfo) -> Result<(), UpdateError> {
        self.kill.effect(|| {
            self.sys.restores.fetch_add(1, Ordering::SeqCst);
            *self.sys.installed.lock().unwrap() = Some(info.previous_version.clone());
            Ok(())
        })
    }
    fn installed_version(&self) -> Result<Version, UpdateError> {
        self.sys
            .installed()
            .ok_or_else(|| UpdateError::Switch("installation state is unknown".into()))
    }
}

struct FakeDownloader {
    sys: Arc<Sys>,
    kill: Arc<Kill>,
    bytes: Vec<u8>,
}
impl Downloader for FakeDownloader {
    fn fetch(&self, _url: &str, dest: &Path) -> Result<(), UpdateError> {
        self.kill.effect(|| {
            self.sys.fetches.fetch_add(1, Ordering::SeqCst);
            fs::write(dest, &self.bytes)?;
            Ok(())
        })
    }
}

struct World {
    tmp: Tmp,
    sys: Arc<Sys>,
    signer: Ed25519Signer,
    manifest_json: String,
    artifact: Vec<u8>,
    previous: Version,
    target: Version,
}

const PROJECT_BYTES: &[u8] = b"user project: must never be touched by the updater";

impl World {
    fn new(tag: &str) -> Self {
        let tmp = Tmp::new(tag);
        fs::create_dir_all(tmp.0.join("projects")).unwrap();
        fs::write(
            tmp.0.join("projects").join("meu filme.capia"),
            PROJECT_BYTES,
        )
        .unwrap();
        let mut seed = [0_u8; 32];
        getrandom::fill(&mut seed).unwrap();
        let signer = Ed25519Signer::from_seed("test-key-1", seed);
        let artifact = format!("installer:0.7.0\n{}", "#".repeat(5000)).into_bytes();
        let (sha, size) = {
            let p = tmp.0.join("probe.bin");
            fs::write(&p, &artifact).unwrap();
            let r = sha256_file(&p).unwrap();
            fs::remove_file(&p).unwrap();
            r
        };
        let m = UpdateManifest {
            schema: MANIFEST_SCHEMA,
            product: PRODUCT.to_owned(),
            version: "0.7.0".to_owned(),
            channel: Channel::Stable,
            artifact: Artifact {
                name: "CapIA_0.7.0_x64-setup.exe".to_owned(),
                url: "https://updates.example.invalid/CapIA_0.7.0_x64-setup.exe".to_owned(),
                sha256: sha,
                size,
            },
            min_version: None,
            notes: "teste".to_owned(),
            rollback: false,
            signature: None,
        }
        .sign(&signer)
        .unwrap();
        Self {
            sys: Sys::new("0.6.0-rc.1"),
            manifest_json: m.to_json().unwrap(),
            signer,
            artifact,
            previous: Version::parse("0.6.0-rc.1").unwrap(),
            target: Version::parse("0.7.0").unwrap(),
            tmp,
        }
    }

    fn state_dir(&self) -> PathBuf {
        self.tmp.0.join("state")
    }

    fn disk_record(&self) -> Option<UpdateRecord> {
        FileStateStore::new(&self.state_dir()).load().unwrap()
    }

    fn assert_projects_intact(&self) {
        let p = self.tmp.0.join("projects").join("meu filme.capia");
        assert_eq!(
            fs::read(p).unwrap(),
            PROJECT_BYTES,
            "o updater tocou num projeto do usuário"
        );
    }

    fn updater(&self, kill: &Arc<Kill>, host: Arc<dyn HostPolicy>, corrupt: bool) -> Updater {
        let running = self
            .sys
            .installed()
            .unwrap_or_else(|| self.previous.clone());
        Updater::new(
            UpdaterConfig {
                state_dir: self.state_dir(),
                current_version: running,
                channel: Channel::Stable,
                max_boot_attempts: 2,
            },
            Arc::new(Ed25519Verifier::new(vec![self.signer.trusted()])),
            Arc::new(FakeSwitcher {
                sys: Arc::clone(&self.sys),
                kill: Arc::clone(kill),
                corrupt_on_switch: corrupt,
            }),
            host,
            Arc::new(KillStore {
                inner: FileStateStore::new(&self.state_dir()),
                kill: Arc::clone(kill),
            }),
        )
        .unwrap()
    }

    fn downloader(&self, kill: &Arc<Kill>) -> FakeDownloader {
        FakeDownloader {
            sys: Arc::clone(&self.sys),
            kill: Arc::clone(kill),
            bytes: self.artifact.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Health {
    Healthy,
    ExplicitFailure,
    /// A nova versão nunca confirma (laço de falhas na inicialização).
    CrashLoop,
}

/// Um "processo" do app: abre, recupera, (opcionalmente) atualiza, faz o health check.
fn process(
    w: &World,
    kill: &Arc<Kill>,
    health: Health,
    seen: &mut Vec<RecoveryReport>,
) -> Result<(), UpdateError> {
    kill.new_process();
    let mut up = w.updater(kill, Arc::new(AlwaysAllow), false);
    let running = w.sys.installed().expect("installation known");
    seen.push(up.recover()?);
    match up.state() {
        UpdateState::Idle | UpdateState::Confirmed => {
            if running < w.target {
                match up.start(w.manifest_json.as_bytes(), false, &w.downloader(kill)) {
                    Ok(()) => {}
                    Err(UpdateError::Rejected(_)) => return Ok(()),
                    Err(e) => return Err(e),
                }
                up.advance()?;
            }
        }
        UpdateState::Downloaded | UpdateState::Verified | UpdateState::Staged => {
            up.advance()?;
        }
        UpdateState::Switched => {}
    }
    // o instalador relançaria o app: o health check é do processo seguinte, já rodando a nova versão
    if up.state() == UpdateState::Switched && running == w.target {
        match health {
            Health::Healthy => up.confirm_healthy()?,
            Health::ExplicitFailure => up.report_unhealthy("startup health check failed")?,
            Health::CrashLoop => {}
        }
    }
    Ok(())
}

fn terminal_ok(w: &World) -> bool {
    let rec = w.disk_record();
    w.sys.installed().as_ref() == Some(&w.target)
        && rec.is_some_and(|r| r.state == UpdateState::Confirmed)
}

fn terminal_rolled_back(w: &World) -> bool {
    let Some(r) = w.disk_record() else {
        return false;
    };
    w.sys.installed().as_ref() == Some(&w.previous)
        && r.state == UpdateState::Idle
        && r.rejected_versions.contains(&w.target.to_string())
        && matches!(r.last_outcome, Some(Outcome::RolledBack { .. }))
}

/// Executa o cenário inteiro com uma morte em `at` e devolve o que foi observado.
fn drive(
    tag: &str,
    health: Health,
    at: usize,
    done: fn(&World) -> bool,
) -> (World, Arc<Kill>, Vec<RecoveryReport>) {
    let w = World::new(tag);
    let kill = Kill::new(at);
    let mut seen = Vec::new();
    for round in 0..16 {
        match process(&w, &kill, health, &mut seen) {
            Ok(()) => {}
            Err(UpdateError::Killed) => {
                // logo após a morte o disco precisa continuar legível (escrita atômica)
                let _ = w.disk_record();
            }
            Err(e) => panic!("kill@{at} round {round}: unexpected error {e}"),
        }
        if done(&w) {
            // próxima abertura do app: `recover()` limpa o que a morte deixou para trás
            let _ = process(&w, &kill, health, &mut seen);
            assert!(
                done(&w),
                "kill@{at}: terminal state must be stable across a restart"
            );
            return (w, kill, seen);
        }
    }
    panic!(
        "kill@{at} ({health:?}): did not converge; installed={:?} record={:?}",
        w.sys.installed(),
        w.disk_record()
    );
}

fn count_effects(tag: &str, health: Health, done: fn(&World) -> bool) -> usize {
    let (_, kill, _) = drive(tag, health, usize::MAX, done);
    kill.effects()
}

fn assert_clean(w: &World) {
    w.assert_projects_intact();
    let dl = w.state_dir().join("downloads");
    assert!(
        !dl.exists() || fs::read_dir(&dl).unwrap().count() == 0,
        "downloads left behind in a resting state"
    );
}

// ---- testes ---------------------------------------------------------------------------------------

#[test]
fn every_kill_point_of_a_healthy_update_converges_to_confirmed() {
    let n = count_effects("clean", Health::Healthy, terminal_ok);
    assert!(
        n >= 8,
        "the cycle must expose many kill points (found {n} effects)"
    );
    let mut all_reports: BTreeSet<String> = BTreeSet::new();
    let mut fired = 0;
    for at in 0..(2 * n) {
        let (w, kill, seen) = drive("healthy", Health::Healthy, at, terminal_ok);
        assert!(
            kill.fired(),
            "kill point {at} never fired (test would be vacuous)"
        );
        fired += 1;
        assert!(terminal_ok(&w));
        assert_clean(&w);
        // a versão nova é exatamente a do manifesto, instalada uma única vez de forma observável
        assert_eq!(
            w.sys.restores.load(Ordering::SeqCst),
            0,
            "kill@{at}: no rollback expected"
        );
        for r in seen {
            all_reports.insert(format!("{r:?}"));
        }
    }
    assert_eq!(fired, 2 * n);
    // a recuperação realmente exercitou os ramos relevantes
    for needle in [
        "Resume(Downloaded)",
        "Resume(Verified)",
        "Resume(Staged)",
        "SwitchObserved",
    ] {
        assert!(
            all_reports.iter().any(|r| r == needle),
            "recovery branch {needle} never exercised: {all_reports:?}"
        );
    }
}

#[test]
fn every_kill_point_with_explicit_health_failure_ends_rolled_back() {
    let n = count_effects("rb-clean", Health::ExplicitFailure, terminal_rolled_back);
    assert!(n >= 10);
    let mut rollback_resumed = false;
    for at in 0..(2 * n) {
        let (w, kill, seen) = drive("rb", Health::ExplicitFailure, at, terminal_rolled_back);
        assert!(kill.fired(), "kill point {at} did not fire");
        assert!(terminal_rolled_back(&w));
        assert_clean(&w);
        assert!(w.sys.restores.load(Ordering::SeqCst) <= 2);
        rollback_resumed |= seen
            .iter()
            .any(|r| matches!(r, RecoveryReport::RolledBack { .. }));
        // a versão ruim não volta sozinha: nova checagem é recusada pela política
        let mut up = w.updater(&Kill::never(), Arc::new(AlwaysAllow), false);
        let e = up
            .start(
                w.manifest_json.as_bytes(),
                false,
                &w.downloader(&Kill::never()),
            )
            .unwrap_err();
        assert!(matches!(e, UpdateError::Rejected(_)), "kill@{at}: {e}");
    }
    assert!(
        rollback_resumed,
        "a kill during rollback must be completed by recover()"
    );
}

#[test]
fn every_kill_point_in_a_crash_loop_ends_rolled_back_automatically() {
    let n = count_effects("loop-clean", Health::CrashLoop, terminal_rolled_back);
    assert!(n >= 12);
    for at in 0..(2 * n) {
        let (w, kill, _) = drive("loop", Health::CrashLoop, at, terminal_rolled_back);
        assert!(kill.fired());
        assert!(terminal_rolled_back(&w));
        assert_clean(&w);
    }
}

#[test]
fn an_installer_that_dies_midway_leaves_unknown_state_and_recovery_rolls_back() {
    let w = World::new("partial");
    let kill = Kill::never();
    let mut up = w.updater(&kill, Arc::new(AlwaysAllow), true);
    up.start(w.manifest_json.as_bytes(), false, &w.downloader(&kill))
        .unwrap();
    let err = up.advance().unwrap_err();
    assert!(matches!(err, UpdateError::Killed));
    assert!(
        w.sys.installed().is_none(),
        "partial install must be unknown"
    );
    drop(up);
    // novo processo: sem versão conhecida, assume a anterior só para abrir
    let mut up = w.updater(&Kill::never(), Arc::new(AlwaysAllow), false);
    let rep = up.recover().unwrap();
    assert!(matches!(rep, RecoveryReport::RolledBack { .. }), "{rep:?}");
    assert_eq!(w.sys.installed(), Some(w.previous.clone()));
    assert_eq!(up.state(), UpdateState::Idle);
    w.assert_projects_intact();
}

#[test]
fn a_corrupt_download_never_reaches_staging_or_the_installer() {
    let w = World::new("corrupt");
    let kill = Kill::never();
    let mut up = w.updater(&kill, Arc::new(AlwaysAllow), false);
    let mut bad = w.artifact.clone();
    bad[40] ^= 0xFF;
    let dl = FakeDownloader {
        sys: Arc::clone(&w.sys),
        kill: Arc::clone(&kill),
        bytes: bad,
    };
    up.start(w.manifest_json.as_bytes(), false, &dl).unwrap();
    let e = up.verify_download().unwrap_err();
    assert!(matches!(e, UpdateError::HashMismatch(_)));
    assert_eq!(up.state(), UpdateState::Idle);
    assert_eq!(w.sys.switches.load(Ordering::SeqCst), 0);
    assert!(matches!(
        up.record().last_outcome,
        Some(Outcome::Failed { .. })
    ));
    // truncado (tamanho diferente) também
    let dl = FakeDownloader {
        sys: Arc::clone(&w.sys),
        kill: Arc::clone(&kill),
        bytes: w.artifact[..100].to_vec(),
    };
    up.start(w.manifest_json.as_bytes(), false, &dl).unwrap();
    assert!(up.verify_download().is_err());
    assert_eq!(w.sys.switches.load(Ordering::SeqCst), 0);
    w.assert_projects_intact();
}

#[test]
fn a_staged_file_tampered_with_before_the_switch_is_refused() {
    let w = World::new("tamper");
    let kill = Kill::never();
    let mut up = w.updater(&kill, Arc::new(AlwaysAllow), false);
    up.start(w.manifest_json.as_bytes(), false, &w.downloader(&kill))
        .unwrap();
    up.verify_download().unwrap();
    up.stage().unwrap();
    let staged = up.record().staged_path.clone().unwrap();
    fs::write(&staged, b"installer:9.9.9 malicious").unwrap();
    let e = up.switch().unwrap_err();
    assert!(matches!(e, UpdateError::HashMismatch(_)), "{e}");
    assert_eq!(
        w.sys.switches.load(Ordering::SeqCst),
        0,
        "installer must not run on a tampered file"
    );
}

#[test]
fn unsigned_forged_or_downgrade_manifests_never_download_or_install() {
    let w = World::new("forged");
    let kill = Kill::never();
    let mut up = w.updater(&kill, Arc::new(AlwaysAllow), false);
    let dl = w.downloader(&kill);
    // sem assinatura
    let mut v: serde_json::Value = serde_json::from_str(&w.manifest_json).unwrap();
    v.as_object_mut().unwrap().remove("signature");
    assert!(up.start(v.to_string().as_bytes(), false, &dl).is_err());
    // adulterado depois de assinado
    let forged = w.manifest_json.replace("0.7.0", "9.9.9");
    assert!(up.start(forged.as_bytes(), false, &dl).is_err());
    // assinado por outra chave
    let other = Ed25519Signer::from_seed("test-key-1", [7_u8; 32]);
    let m = UpdateManifest::parse_and_verify(
        w.manifest_json.as_bytes(),
        &Ed25519Verifier::new(vec![w.signer.trusted()]),
    )
    .unwrap()
    .sign(&other)
    .unwrap();
    assert!(
        up.start(m.to_json().unwrap().as_bytes(), false, &dl)
            .is_err()
    );
    assert_eq!(
        w.sys.fetches.load(Ordering::SeqCst),
        0,
        "nothing may be downloaded before verification"
    );
    assert_eq!(w.sys.switches.load(Ordering::SeqCst), 0);
    assert_eq!(up.state(), UpdateState::Idle);
    // downgrade silencioso: sistema já está em 0.7.0 e o manifesto é 0.7.0 → "já atualizado", nunca instala
    *w.sys.installed.lock().unwrap() = Some(Version::parse("0.8.0").unwrap());
    let mut up = w.updater(&kill, Arc::new(AlwaysAllow), false);
    let e = up
        .start(w.manifest_json.as_bytes(), false, &dl)
        .unwrap_err();
    assert!(matches!(e, UpdateError::Rejected(m) if m.contains("downgrade")));
    assert_eq!(w.sys.fetches.load(Ordering::SeqCst), 0);
}

#[test]
fn explicit_rollback_manifest_allows_going_back_only_when_requested() {
    let w = World::new("rbm");
    *w.sys.installed.lock().unwrap() = Some(Version::parse("0.8.0").unwrap());
    let mut m: serde_json::Value = serde_json::from_str(&w.manifest_json).unwrap();
    m["rollback"] = serde_json::Value::Bool(true);
    let signed = serde_json::from_value::<UpdateManifest>(m)
        .unwrap()
        .sign(&w.signer)
        .unwrap()
        .to_json()
        .unwrap();
    let kill = Kill::never();
    let mut up = w.updater(&kill, Arc::new(AlwaysAllow), false);
    assert!(
        up.start(signed.as_bytes(), false, &w.downloader(&kill))
            .is_err()
    );
    up.start(signed.as_bytes(), true, &w.downloader(&kill))
        .unwrap();
    assert_eq!(up.advance().unwrap(), UpdateState::Switched);
    assert_eq!(w.sys.installed(), Some(Version::parse("0.7.0").unwrap()));
}

struct Gate {
    open: AtomicBool,
    checkpoints: AtomicUsize,
    needs_checkpoint: bool,
}
impl HostPolicy for Gate {
    fn gate(&self) -> SwitchGate {
        if self.open.load(Ordering::SeqCst) {
            SwitchGate::Allow
        } else if self.needs_checkpoint {
            SwitchGate::CheckpointRequired
        } else {
            SwitchGate::Defer("an AI Run is active".into())
        }
    }
    fn checkpoint(&self) -> Result<(), String> {
        self.checkpoints.fetch_add(1, Ordering::SeqCst);
        self.open.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn an_active_ai_run_defers_the_switch_and_nothing_is_corrupted() {
    let w = World::new("gate");
    let kill = Kill::never();
    let gate = Arc::new(Gate {
        open: AtomicBool::new(false),
        checkpoints: AtomicUsize::new(0),
        needs_checkpoint: false,
    });
    let mut up = w.updater(&kill, gate.clone(), false);
    up.start(w.manifest_json.as_bytes(), false, &w.downloader(&kill))
        .unwrap();
    let e = up.advance().unwrap_err();
    assert!(matches!(e, UpdateError::Deferred(_)), "{e}");
    assert_eq!(
        up.state(),
        UpdateState::Staged,
        "deferred updates wait staged, ready"
    );
    assert_eq!(w.sys.switches.load(Ordering::SeqCst), 0);
    // o app reabre ainda com a Run ativa: continua adiado
    drop(up);
    let mut up = w.updater(&kill, gate.clone(), false);
    assert_eq!(
        up.recover().unwrap(),
        RecoveryReport::Resume(UpdateState::Staged)
    );
    assert!(up.advance().is_err());
    assert_eq!(w.sys.switches.load(Ordering::SeqCst), 0);
    // Run termina: o switch acontece
    gate.open.store(true, Ordering::SeqCst);
    assert_eq!(up.advance().unwrap(), UpdateState::Switched);
    w.assert_projects_intact();
}

#[test]
fn checkpoint_policy_checkpoints_then_switches() {
    let w = World::new("ckpt");
    let kill = Kill::never();
    let gate = Arc::new(Gate {
        open: AtomicBool::new(false),
        checkpoints: AtomicUsize::new(0),
        needs_checkpoint: true,
    });
    let mut up = w.updater(&kill, gate.clone(), false);
    up.start(w.manifest_json.as_bytes(), false, &w.downloader(&kill))
        .unwrap();
    assert_eq!(up.advance().unwrap(), UpdateState::Switched);
    assert_eq!(gate.checkpoints.load(Ordering::SeqCst), 1);
}
