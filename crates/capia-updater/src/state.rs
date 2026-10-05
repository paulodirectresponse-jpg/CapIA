//! Máquina de estados do update, **persistida em disco** e recuperável após morte do processo em
//! qualquer ponto.
//!
//! `Idle → Downloaded → Verified → Staged → Switched → Confirmed`
//!
//! * `Downloaded`: artefato baixado (manifesto já verificado por assinatura **antes** do download).
//! * `Verified`: tamanho + SHA-256 do artefato conferem com o manifesto assinado.
//! * `Staged`: cópia verificada em `staging/`, pronta para o instalador (Switcher).
//! * `Switched`: o instalador substituiu os arquivos (verdade: `Switcher::installed_version`).
//! * `Confirmed`: a nova versão passou no health check de inicialização (marcador `health.json`).
//!
//! Não confirmou (health explícito falhou ou inicializações demais sem confirmar) ⇒ **rollback
//! automático** (`Switcher::restore_previous`), a versão é lembrada em `rejected_versions` e não volta
//! sozinha. Atualizar durante escrita crítica / AI Run ativa é decisão do host (`HostPolicy`):
//! adiar ou fazer checkpoint. Este módulo só toca `state_dir` — nunca projetos do usuário.

use crate::error::UpdateError;
use crate::manifest::{Channel, UpdateManifest};
use crate::verify::Verifier;
use crate::version::{self, Decision, UpdateKind, Version};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const STATE_FILE: &str = "update-state.json";
pub const HEALTH_FILE: &str = "health.json";
const DOWNLOADS: &str = "downloads";
const STAGING: &str = "staging";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateState {
    Idle,
    Downloaded,
    Verified,
    Staged,
    Switched,
    Confirmed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Updated {
        from: String,
        to: String,
    },
    RolledBack {
        from: String,
        to: String,
        reason: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateRecord {
    pub schema: u32,
    pub state: UpdateState,
    /// JSON **recebido** do manifesto (reverificado a cada recuperação; nunca se confia no disco).
    pub manifest_json: Option<String>,
    pub kind: Option<UpdateKind>,
    pub previous_version: Option<String>,
    pub target_version: Option<String>,
    pub downloaded_path: Option<PathBuf>,
    pub staged_path: Option<PathBuf>,
    pub boot_attempts: u32,
    pub rolling_back: bool,
    pub rollback_reason: Option<String>,
    pub rejected_versions: Vec<String>,
    pub last_outcome: Option<Outcome>,
}

impl Default for UpdateRecord {
    fn default() -> Self {
        Self {
            schema: 1,
            state: UpdateState::Idle,
            manifest_json: None,
            kind: None,
            previous_version: None,
            target_version: None,
            downloaded_path: None,
            staged_path: None,
            boot_attempts: 0,
            rolling_back: false,
            rollback_reason: None,
            rejected_versions: Vec::new(),
            last_outcome: None,
        }
    }
}

// ---- pontos de extensão do host ------------------------------------------------------------------

pub trait StateStore: Send + Sync + std::fmt::Debug {
    fn load(&self) -> Result<Option<UpdateRecord>, UpdateError>;
    fn save(&self, rec: &UpdateRecord) -> Result<(), UpdateError>;
}

/// Baixa `url` para `dest` (o crate não tem código de rede; o host injeta o cliente).
pub trait Downloader: Send + Sync {
    fn fetch(&self, url: &str, dest: &Path) -> Result<(), UpdateError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackInfo {
    pub previous_version: Version,
    pub failed_version: Version,
}

/// Troca real dos arquivos instalados — feita pelo instalador da plataforma (NSIS silencioso).
pub trait Switcher: Send + Sync {
    /// Coloca o artefato verificado no staging e devolve o caminho que `switch` receberá.
    /// Também retém o instalador da versão anterior para o rollback.
    fn stage(&self, verified_artifact: &Path, staging_dir: &Path) -> Result<PathBuf, UpdateError>;
    /// Executa a troca (instalador silencioso). Idempotente: repetir sobre a mesma versão é inofensivo.
    fn switch(&self, staged: &Path) -> Result<(), UpdateError>;
    /// Restaura a versão anterior (reexecuta o instalador retido).
    fn restore_previous(&self, info: &RollbackInfo) -> Result<(), UpdateError>;
    /// Versão **efetivamente instalada** agora (a verdade para a recuperação).
    fn installed_version(&self) -> Result<Version, UpdateError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitchGate {
    Allow,
    /// Escrita crítica / Run de IA ativa: não trocar agora.
    Defer(String),
    /// O host precisa fazer checkpoint antes (`HostPolicy::checkpoint`); depois reavalia uma vez.
    CheckpointRequired,
}

/// Política do host: **nunca atualizar durante escrita crítica ou AI Run ativa** — adiar ou fazer checkpoint.
pub trait HostPolicy: Send + Sync {
    fn gate(&self) -> SwitchGate;
    fn checkpoint(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Política padrão restritiva: sempre pode (usada por ferramentas sem Runs).
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysAllow;
impl HostPolicy for AlwaysAllow {
    fn gate(&self) -> SwitchGate {
        SwitchGate::Allow
    }
}

// ---- armazenamento em arquivo ---------------------------------------------------------------------

/// Escrita atômica: arquivo temporário + fsync + rename (+ fsync do diretório quando possível).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), UpdateError> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    #[cfg(unix)]
    if let Some(dir) = path.parent()
        && let Ok(d) = File::open(dir)
    {
        let _ = d.sync_all();
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct FileStateStore {
    path: PathBuf,
}

impl FileStateStore {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            path: state_dir.join(STATE_FILE),
        }
    }
}

impl StateStore for FileStateStore {
    fn load(&self) -> Result<Option<UpdateRecord>, UpdateError> {
        match fs::read(&self.path) {
            // arquivo ilegível ⇒ trata como sem estado (escrita atômica torna isto raro; o app nunca trava)
            Ok(b) => Ok(serde_json::from_slice(&b).ok()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn save(&self, rec: &UpdateRecord) -> Result<(), UpdateError> {
        let json = serde_json::to_vec_pretty(rec).map_err(|e| UpdateError::Io(e.to_string()))?;
        write_atomic(&self.path, &json)
    }
}

// ---- núcleo ---------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct UpdaterConfig {
    pub state_dir: PathBuf,
    /// Versão do binário **em execução**.
    pub current_version: Version,
    pub channel: Channel,
    /// Inicializações toleradas sem confirmação antes do rollback automático (padrão 2).
    pub max_boot_attempts: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryReport {
    Nothing,
    /// O fluxo pode continuar em `state` (chame `advance`).
    Resume(UpdateState),
    /// A troca foi observada (o processo morreu antes de gravar `Switched`).
    SwitchObserved,
    /// O instalador não chegou a trocar: volta a `Staged` para nova tentativa.
    SwitchNotApplied,
    /// Nova versão instalada, mas o processo em execução ainda é o antigo (aguarda reinício).
    AwaitingRestart,
    /// Nova versão iniciou (tentativa N) e ainda não confirmou saúde.
    AwaitingHealth {
        boot_attempts: u32,
    },
    Confirmed,
    RolledBack {
        reason: String,
    },
    /// Estado inconsistente descartado (ex.: arquivo baixado sumiu).
    Reset(String),
}

pub struct Updater {
    cfg: UpdaterConfig,
    verifier: Arc<dyn Verifier>,
    switcher: Arc<dyn Switcher>,
    host: Arc<dyn HostPolicy>,
    store: Arc<dyn StateStore>,
    rec: UpdateRecord,
}

impl std::fmt::Debug for Updater {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Updater")
            .field("state", &self.rec.state)
            .finish_non_exhaustive()
    }
}

pub fn sha256_file(path: &Path) -> Result<(String, u64), UpdateError> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        total += n as u64;
    }
    Ok((crate::verify::hex_encode(&h.finalize()), total))
}

impl Updater {
    pub fn new(
        cfg: UpdaterConfig,
        verifier: Arc<dyn Verifier>,
        switcher: Arc<dyn Switcher>,
        host: Arc<dyn HostPolicy>,
        store: Arc<dyn StateStore>,
    ) -> Result<Self, UpdateError> {
        fs::create_dir_all(&cfg.state_dir)?;
        let rec = store.load()?.unwrap_or_default();
        Ok(Self {
            cfg,
            verifier,
            switcher,
            host,
            store,
            rec,
        })
    }

    pub fn state(&self) -> UpdateState {
        self.rec.state
    }

    pub fn record(&self) -> &UpdateRecord {
        &self.rec
    }

    fn downloads_dir(&self) -> PathBuf {
        self.cfg.state_dir.join(DOWNLOADS)
    }
    fn staging_dir(&self) -> PathBuf {
        self.cfg.state_dir.join(STAGING)
    }
    fn health_path(&self) -> PathBuf {
        self.cfg.state_dir.join(HEALTH_FILE)
    }

    fn save(&mut self) -> Result<(), UpdateError> {
        self.store.save(&self.rec)
    }

    fn clean_work_dirs(&self) {
        for d in [self.downloads_dir(), self.staging_dir()] {
            let _ = fs::remove_dir_all(&d);
        }
    }

    fn manifest(&self) -> Result<UpdateManifest, UpdateError> {
        let json =
            self.rec.manifest_json.as_deref().ok_or_else(|| {
                UpdateError::WrongState("no manifest in the update record".into())
            })?;
        UpdateManifest::parse_and_verify(json.as_bytes(), self.verifier.as_ref())
    }

    fn previous(&self) -> Result<Version, UpdateError> {
        version::parse(self.rec.previous_version.as_deref().unwrap_or_default())
    }

    fn target(&self) -> Result<Version, UpdateError> {
        version::parse(self.rec.target_version.as_deref().unwrap_or_default())
    }

    fn reset_idle(&mut self, outcome: Option<Outcome>) -> Result<(), UpdateError> {
        let rejected = std::mem::take(&mut self.rec.rejected_versions);
        let last = outcome.or_else(|| self.rec.last_outcome.clone());
        self.rec = UpdateRecord {
            rejected_versions: rejected,
            last_outcome: last,
            ..UpdateRecord::default()
        };
        self.save()?;
        self.clean_work_dirs();
        Ok(())
    }

    /// Só verifica assinatura + política (sem tocar em estado). Para "Verificar atualizações".
    pub fn check(
        &self,
        manifest_bytes: &[u8],
        allow_rollback: bool,
    ) -> Result<(UpdateManifest, Decision), UpdateError> {
        version::check_manifest(
            manifest_bytes,
            self.verifier.as_ref(),
            &self.cfg.current_version,
            self.cfg.channel,
            allow_rollback,
            &self.rec.rejected_versions,
        )
    }

    /// `Idle → Downloaded`. A assinatura é verificada **antes** de qualquer download.
    pub fn start(
        &mut self,
        manifest_bytes: &[u8],
        allow_rollback: bool,
        downloader: &dyn Downloader,
    ) -> Result<(), UpdateError> {
        match self.rec.state {
            UpdateState::Idle => {}
            UpdateState::Confirmed => self.reset_idle(None)?,
            s => {
                return Err(UpdateError::WrongState(format!(
                    "an update is already in progress ({s:?})"
                )));
            }
        }
        let (m, decision) = self.check(manifest_bytes, allow_rollback)?;
        let kind = match decision {
            Decision::Available(k) => k,
            Decision::UpToDate => return Err(UpdateError::Rejected("already up to date".into())),
            Decision::RequiresIntermediate(v) => {
                return Err(UpdateError::Rejected(format!("install version {v} first")));
            }
            Decision::Rejected(r) => return Err(UpdateError::Rejected(r)),
        };
        let json = String::from_utf8(manifest_bytes.to_vec())
            .map_err(|_| UpdateError::Invalid("manifest is not UTF-8".into()))?;
        let dl = self.downloads_dir();
        let _ = fs::remove_dir_all(&dl);
        fs::create_dir_all(&dl)?;
        let part = dl.join(format!("{}.part", m.artifact.name));
        let fin = dl.join(&m.artifact.name);
        downloader.fetch(&m.artifact.url, &part)?;
        fs::rename(&part, &fin)?;
        self.rec.manifest_json = Some(json);
        self.rec.kind = Some(kind);
        self.rec.previous_version = Some(self.cfg.current_version.to_string());
        self.rec.target_version = Some(m.version.clone());
        self.rec.downloaded_path = Some(fin);
        self.rec.state = UpdateState::Downloaded;
        self.save()
    }

    /// `Downloaded → Verified`: tamanho e SHA-256 contra o manifesto **assinado**.
    pub fn verify_download(&mut self) -> Result<(), UpdateError> {
        self.expect(UpdateState::Downloaded)?;
        let m = self.manifest()?;
        let path = self
            .rec
            .downloaded_path
            .clone()
            .ok_or_else(|| UpdateError::WrongState("no downloaded file recorded".into()))?;
        let (sha, size) = sha256_file(&path)?;
        if size != m.artifact.size || sha != m.artifact.sha256 {
            let reason = "downloaded artifact does not match the signed manifest".to_owned();
            self.reset_idle(Some(Outcome::Failed {
                reason: reason.clone(),
            }))?;
            return Err(UpdateError::HashMismatch(reason));
        }
        self.rec.state = UpdateState::Verified;
        self.save()
    }

    /// `Verified → Staged`.
    pub fn stage(&mut self) -> Result<(), UpdateError> {
        self.expect(UpdateState::Verified)?;
        let m = self.manifest()?;
        let src = self
            .rec
            .downloaded_path
            .clone()
            .ok_or_else(|| UpdateError::WrongState("no downloaded file recorded".into()))?;
        let dir = self.staging_dir();
        fs::create_dir_all(&dir)?;
        let staged = self.switcher.stage(&src, &dir)?;
        self.check_hash(&staged, &m)?;
        self.rec.staged_path = Some(staged);
        self.rec.state = UpdateState::Staged;
        self.save()
    }

    fn check_hash(&mut self, path: &Path, m: &UpdateManifest) -> Result<(), UpdateError> {
        let (sha, size) = sha256_file(path)?;
        if size != m.artifact.size || sha != m.artifact.sha256 {
            let reason = "staged artifact does not match the signed manifest".to_owned();
            self.reset_idle(Some(Outcome::Failed {
                reason: reason.clone(),
            }))?;
            return Err(UpdateError::HashMismatch(reason));
        }
        Ok(())
    }

    /// `Staged → Switched`, respeitando a política do host (adiar/checkpoint). Reconfere o hash antes de trocar.
    pub fn switch(&mut self) -> Result<(), UpdateError> {
        self.expect(UpdateState::Staged)?;
        match self.host.gate() {
            SwitchGate::Allow => {}
            SwitchGate::Defer(r) => return Err(UpdateError::Deferred(r)),
            SwitchGate::CheckpointRequired => {
                self.host.checkpoint().map_err(UpdateError::Deferred)?;
                match self.host.gate() {
                    SwitchGate::Allow => {}
                    SwitchGate::Defer(r) => return Err(UpdateError::Deferred(r)),
                    SwitchGate::CheckpointRequired => {
                        return Err(UpdateError::Deferred(
                            "checkpoint did not clear the gate".into(),
                        ));
                    }
                }
            }
        }
        let m = self.manifest()?;
        let staged = self
            .rec
            .staged_path
            .clone()
            .ok_or_else(|| UpdateError::WrongState("no staged file recorded".into()))?;
        self.check_hash(&staged, &m)?;
        self.switcher.switch(&staged)?;
        self.rec.state = UpdateState::Switched;
        self.rec.boot_attempts = 0;
        self.save()
    }

    /// Avança o fluxo a partir do estado atual até `Switched` (ou até falhar/adiar).
    pub fn advance(&mut self) -> Result<UpdateState, UpdateError> {
        loop {
            match self.rec.state {
                UpdateState::Downloaded => self.verify_download()?,
                UpdateState::Verified => self.stage()?,
                UpdateState::Staged => self.switch()?,
                s => return Ok(s),
            }
        }
    }

    /// `Switched → Confirmed`: chamada pelo app **depois** do health check de inicialização.
    pub fn confirm_healthy(&mut self) -> Result<(), UpdateError> {
        self.expect(UpdateState::Switched)?;
        let target = self.target()?;
        if version::compare(&self.cfg.current_version, &target) != std::cmp::Ordering::Equal {
            return Err(UpdateError::WrongState(format!(
                "running {} but the update target is {target}",
                self.cfg.current_version
            )));
        }
        let marker = serde_json::json!({"version": target.to_string()}).to_string();
        write_atomic(&self.health_path(), marker.as_bytes())?;
        self.finish_confirm()
    }

    fn finish_confirm(&mut self) -> Result<(), UpdateError> {
        let outcome = Outcome::Updated {
            from: self.rec.previous_version.clone().unwrap_or_default(),
            to: self.rec.target_version.clone().unwrap_or_default(),
        };
        self.rec.state = UpdateState::Confirmed;
        self.rec.last_outcome = Some(outcome);
        self.save()?;
        self.clean_work_dirs();
        Ok(())
    }

    /// O health check de inicialização falhou ⇒ rollback imediato.
    pub fn report_unhealthy(&mut self, reason: &str) -> Result<(), UpdateError> {
        self.expect(UpdateState::Switched)?;
        self.begin_rollback(reason)
    }

    fn begin_rollback(&mut self, reason: &str) -> Result<(), UpdateError> {
        self.rec.rolling_back = true;
        self.rec.rollback_reason = Some(reason.to_owned());
        self.save()?;
        self.finish_rollback()
    }

    fn finish_rollback(&mut self) -> Result<(), UpdateError> {
        let previous = self.previous()?;
        let failed = self.target()?;
        let installed = self.switcher.installed_version();
        let already = matches!(&installed, Ok(v) if version::compare(v, &previous) == std::cmp::Ordering::Equal);
        if !already {
            self.switcher.restore_previous(&RollbackInfo {
                previous_version: previous.clone(),
                failed_version: failed.clone(),
            })?;
        }
        let reason = self.rec.rollback_reason.clone().unwrap_or_default();
        if !self.rec.rejected_versions.contains(&failed.to_string()) {
            self.rec.rejected_versions.push(failed.to_string());
        }
        self.reset_idle(Some(Outcome::RolledBack {
            from: failed.to_string(),
            to: previous.to_string(),
            reason,
        }))
    }

    fn expect(&self, s: UpdateState) -> Result<(), UpdateError> {
        if self.rec.state == s {
            Ok(())
        } else {
            Err(UpdateError::WrongState(format!(
                "expected {s:?}, found {:?}",
                self.rec.state
            )))
        }
    }

    fn marker_confirms_target(&self) -> bool {
        let Ok(bytes) = fs::read(self.health_path()) else {
            return false;
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return false;
        };
        v["version"].as_str() == self.rec.target_version.as_deref()
    }

    /// Chamar a **cada** inicialização do app, antes de qualquer outro trabalho. Reconcilia o estado em disco
    /// com a realidade (versão instalada, marcador de saúde) depois de morte em qualquer ponto.
    pub fn recover(&mut self) -> Result<RecoveryReport, UpdateError> {
        match self.rec.state {
            UpdateState::Idle => {
                self.clean_work_dirs();
                Ok(RecoveryReport::Nothing)
            }
            UpdateState::Confirmed => {
                self.clean_work_dirs();
                Ok(RecoveryReport::Confirmed)
            }
            UpdateState::Downloaded | UpdateState::Verified => {
                let ok = self
                    .rec
                    .downloaded_path
                    .as_deref()
                    .is_some_and(Path::is_file)
                    && self.manifest().is_ok();
                if ok {
                    Ok(RecoveryReport::Resume(self.rec.state))
                } else {
                    self.reset_idle(Some(Outcome::Failed {
                        reason: "download state was inconsistent and was discarded".into(),
                    }))?;
                    Ok(RecoveryReport::Reset(
                        "download missing or manifest invalid".into(),
                    ))
                }
            }
            UpdateState::Staged => {
                let previous = self.previous()?;
                let target = self.target()?;
                match self.switcher.installed_version() {
                    Ok(v) if version::compare(&v, &target).is_eq() => {
                        self.rec.state = UpdateState::Switched;
                        self.rec.boot_attempts = 0;
                        self.save()?;
                        Ok(RecoveryReport::SwitchObserved)
                    }
                    Ok(v) if version::compare(&v, &previous).is_eq() => {
                        let staged_ok = self.rec.staged_path.as_deref().is_some_and(Path::is_file);
                        if staged_ok {
                            Ok(RecoveryReport::Resume(UpdateState::Staged))
                        } else {
                            // staging perdido: refaz a partir do download, se ele ainda existir
                            let dl_ok = self
                                .rec
                                .downloaded_path
                                .as_deref()
                                .is_some_and(Path::is_file);
                            if dl_ok {
                                self.rec.state = UpdateState::Verified;
                                self.rec.staged_path = None;
                                self.save()?;
                                Ok(RecoveryReport::Resume(UpdateState::Verified))
                            } else {
                                self.reset_idle(Some(Outcome::Failed {
                                    reason: "staged files were lost".into(),
                                }))?;
                                Ok(RecoveryReport::Reset("staged files lost".into()))
                            }
                        }
                    }
                    // versão instalada desconhecida/ilegível: a troca pode ter sido parcial
                    _ => self.rollback_report(
                        "installed version is unknown after an interrupted switch",
                    ),
                }
            }
            UpdateState::Switched => {
                if self.rec.rolling_back {
                    self.finish_rollback()?;
                    return Ok(RecoveryReport::RolledBack {
                        reason: "rollback completed after interruption".into(),
                    });
                }
                let previous = self.previous()?;
                let target = self.target()?;
                match self.switcher.installed_version() {
                    Ok(v) if version::compare(&v, &target).is_eq() => {
                        if self.marker_confirms_target() {
                            self.finish_confirm()?;
                            return Ok(RecoveryReport::Confirmed);
                        }
                        if version::compare(&self.cfg.current_version, &target).is_ne() {
                            return Ok(RecoveryReport::AwaitingRestart);
                        }
                        self.rec.boot_attempts += 1;
                        self.save()?;
                        if self.rec.boot_attempts > self.cfg.max_boot_attempts {
                            return self.rollback_report(
                                "the new version did not confirm a healthy start",
                            );
                        }
                        Ok(RecoveryReport::AwaitingHealth {
                            boot_attempts: self.rec.boot_attempts,
                        })
                    }
                    Ok(v) if version::compare(&v, &previous).is_eq() => {
                        // o instalador não chegou a trocar: nova tentativa a partir do staging
                        self.rec.state = UpdateState::Staged;
                        self.rec.boot_attempts = 0;
                        self.save()?;
                        Ok(RecoveryReport::SwitchNotApplied)
                    }
                    _ => self.rollback_report("installed version is unknown"),
                }
            }
        }
    }

    fn rollback_report(&mut self, reason: &str) -> Result<RecoveryReport, UpdateError> {
        self.begin_rollback(reason)?;
        Ok(RecoveryReport::RolledBack {
            reason: reason.to_owned(),
        })
    }
}
