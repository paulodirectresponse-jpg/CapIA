//! `Switcher` real: a troca de arquivos é feita pelo **instalador da plataforma** (NSIS silencioso, por
//! usuário, sem elevação). O updater nunca sobrescreve binários instalados por conta própria.
//!
//! * `stage`: copia o artefato JÁ verificado para o staging por `*.part` + `rename` (nunca um
//!   instalador pela metade); o instalador da versão anterior fica retido em `retained_dir`.
//! * `switch`: executa o instalador (`/S`). Dois modos: **esperar** (testes/ferramentas) ou **destacado**
//!   (o app lança o instalador e sai; quem confirma é a próxima partida, via `recover()` + marcador de
//!   saúde — o app em execução não pode ser sobrescrito no Windows).
//! * `restore_previous`: reexecuta o instalador retido da versão anterior. Sem instalador retido a
//!   restauração é recusada com erro claro (procedimento manual documentado), nunca "fingida".
//! * `installed_version`: a verdade vem de um sensor injetado (registro de desinstalação / `--version`
//!   do executável instalado), nunca do que o updater acha que fez.

use crate::error::UpdateError;
use crate::state::{RollbackInfo, Switcher, write_atomic};
use crate::version::Version;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Executa um programa (o instalador). Injetável: os testes simulam o instalador sem processo.
pub trait CommandRunner: Send + Sync {
    /// Espera o término e devolve o código de saída.
    fn run_wait(
        &self,
        program: &Path,
        args: &[String],
        timeout: Duration,
    ) -> Result<i32, UpdateError>;
    /// Lança sem esperar (o chamador vai sair).
    fn spawn_detached(&self, program: &Path, args: &[String]) -> Result<(), UpdateError>;
}

/// Execução de processo real (sem shell: argumentos como vetor).
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn run_wait(
        &self,
        program: &Path,
        args: &[String],
        timeout: Duration,
    ) -> Result<i32, UpdateError> {
        let mut child = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| UpdateError::Switch(format!("cannot start the installer: {e}")))?;
        let t0 = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(st)) => return Ok(st.code().unwrap_or(-1)),
                Ok(None) if t0.elapsed() > timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(UpdateError::Switch("the installer timed out".into()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(e) => {
                    return Err(UpdateError::Switch(format!(
                        "waiting for the installer: {e}"
                    )));
                }
            }
        }
    }

    fn spawn_detached(&self, program: &Path, args: &[String]) -> Result<(), UpdateError> {
        std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| UpdateError::Switch(format!("cannot start the installer: {e}")))
    }
}

type VersionProbe = dyn Fn() -> Result<Version, UpdateError> + Send + Sync;

pub struct InstallerSwitcher {
    retained_dir: PathBuf,
    args: Vec<String>,
    timeout: Duration,
    detached: bool,
    runner: Arc<dyn CommandRunner>,
    probe: Arc<VersionProbe>,
}

impl core::fmt::Debug for InstallerSwitcher {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InstallerSwitcher")
            .field("retained_dir", &self.retained_dir)
            .field("detached", &self.detached)
            .finish_non_exhaustive()
    }
}

/// Nome retido de um instalador: só `[0-9A-Za-z.+-]` da versão (nada de separador de caminho).
fn retained_name(v: &Version) -> String {
    let safe: String = v
        .to_string()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}-setup.exe")
}

impl InstallerSwitcher {
    /// `retained_dir`: onde ficam os instaladores das versões já instaladas (rollback).
    /// `probe`: lê a versão efetivamente instalada.
    pub fn new(
        retained_dir: impl Into<PathBuf>,
        probe: impl Fn() -> Result<Version, UpdateError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            retained_dir: retained_dir.into(),
            args: vec!["/S".to_owned()],
            timeout: Duration::from_secs(600),
            detached: false,
            runner: Arc::new(ProcessRunner),
            probe: Arc::new(probe),
        }
    }

    pub fn with_runner(mut self, r: Arc<dyn CommandRunner>) -> Self {
        self.runner = r;
        self
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub fn with_timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }

    /// O app lança o instalador e sai; a confirmação acontece na próxima partida.
    pub fn detached(mut self, d: bool) -> Self {
        self.detached = d;
        self
    }

    /// Guarda o instalador de uma versão **instalada com sucesso** (cópia atômica) para o rollback.
    pub fn retain(&self, installer: &Path, version: &Version) -> Result<PathBuf, UpdateError> {
        std::fs::create_dir_all(&self.retained_dir)?;
        let dest = self.retained_dir.join(retained_name(version));
        let bytes = std::fs::read(installer)?;
        write_atomic(&dest, &bytes)?;
        Ok(dest)
    }

    pub fn retained_for(&self, version: &Version) -> Option<PathBuf> {
        let p = self.retained_dir.join(retained_name(version));
        p.is_file().then_some(p)
    }

    fn run(&self, installer: &Path) -> Result<(), UpdateError> {
        if !installer.is_file() {
            return Err(UpdateError::Switch(format!(
                "installer not found: {}",
                installer.display()
            )));
        }
        if self.detached {
            return self.runner.spawn_detached(installer, &self.args);
        }
        match self.runner.run_wait(installer, &self.args, self.timeout)? {
            0 => Ok(()),
            code => Err(UpdateError::Switch(format!(
                "the installer exited with code {code}"
            ))),
        }
    }
}

impl Switcher for InstallerSwitcher {
    fn stage(&self, verified_artifact: &Path, staging_dir: &Path) -> Result<PathBuf, UpdateError> {
        std::fs::create_dir_all(staging_dir)?;
        let name = verified_artifact
            .file_name()
            .ok_or_else(|| UpdateError::Invalid("artifact has no file name".into()))?;
        let dest = staging_dir.join(name);
        let bytes = std::fs::read(verified_artifact)?;
        // `*.part` + rename: um instalador pela metade nunca fica com o nome final
        write_atomic(&dest, &bytes)?;
        Ok(dest)
    }

    fn switch(&self, staged: &Path) -> Result<(), UpdateError> {
        // o instalador da versão que está instalada AGORA fica retido antes de ser substituída
        // (quando o updater a conhece: `retain` é chamado após cada troca confirmada)
        self.run(staged)
    }

    fn restore_previous(&self, info: &RollbackInfo) -> Result<(), UpdateError> {
        let Some(prev) = self.retained_for(&info.previous_version) else {
            return Err(UpdateError::Switch(format!(
                "no retained installer for {}: restore it manually (see docs/phase6/IMPL_DESKTOP_DISTRIBUTION.md)",
                info.previous_version
            )));
        };
        self.run(&prev)
    }

    fn installed_version(&self) -> Result<Version, UpdateError> {
        (self.probe)()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// "Instalador" simulado: lê a versão do arquivo do instalador e a grava como instalada.
    struct FakeRunner {
        installed: Arc<Mutex<Option<String>>>,
        calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
        exit: i32,
        detached_calls: Mutex<u32>,
    }

    impl CommandRunner for FakeRunner {
        fn run_wait(
            &self,
            program: &Path,
            args: &[String],
            _t: Duration,
        ) -> Result<i32, UpdateError> {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_path_buf(), args.to_vec()));
            if self.exit == 0 {
                let v = std::fs::read_to_string(program).unwrap();
                *self.installed.lock().unwrap() = Some(v.trim().to_owned());
            }
            Ok(self.exit)
        }
        fn spawn_detached(&self, program: &Path, args: &[String]) -> Result<(), UpdateError> {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_path_buf(), args.to_vec()));
            *self.detached_calls.lock().unwrap() += 1;
            Ok(())
        }
    }

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "capia-switcher-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn rig(
        exit: i32,
    ) -> (
        InstallerSwitcher,
        Arc<FakeRunner>,
        Arc<Mutex<Option<String>>>,
        PathBuf,
    ) {
        let dir = tmp("rig");
        let installed = Arc::new(Mutex::new(Some("0.6.0".to_owned())));
        let runner = Arc::new(FakeRunner {
            installed: Arc::clone(&installed),
            calls: Mutex::new(vec![]),
            exit,
            detached_calls: Mutex::new(0),
        });
        let inst = Arc::clone(&installed);
        let sw = InstallerSwitcher::new(dir.join("retained"), move || {
            inst.lock()
                .unwrap()
                .clone()
                .and_then(|v| Version::parse(&v).ok())
                .ok_or_else(|| UpdateError::Switch("unknown".into()))
        })
        .with_runner(runner.clone());
        (sw, runner, installed, dir)
    }

    fn installer(dir: &Path, name: &str, version: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("{version}\n")).unwrap();
        p
    }

    #[test]
    fn stage_copies_atomically_and_never_leaves_a_part_file() {
        let (sw, _r, _i, dir) = rig(0);
        let art = installer(&dir, "CapIA_0.7.0_x64-setup.exe", "0.7.0");
        let staged = sw.stage(&art, &dir.join("staging")).unwrap();
        assert_eq!(
            std::fs::read(&staged).unwrap(),
            std::fs::read(&art).unwrap()
        );
        let parts: Vec<_> = std::fs::read_dir(dir.join("staging"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(parts.is_empty());
    }

    #[test]
    fn switch_runs_the_silent_installer_and_the_probe_reports_the_truth() {
        let (sw, r, _i, dir) = rig(0);
        let art = installer(&dir, "new-setup.exe", "0.7.0");
        let staged = sw.stage(&art, &dir.join("staging")).unwrap();
        assert_eq!(
            sw.installed_version().unwrap(),
            Version::parse("0.6.0").unwrap()
        );
        sw.switch(&staged).unwrap();
        assert_eq!(
            sw.installed_version().unwrap(),
            Version::parse("0.7.0").unwrap()
        );
        let calls = r.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, vec!["/S".to_owned()]);
        // idempotente: repetir sobre a mesma versão é inofensivo
        drop(calls);
        sw.switch(&staged).unwrap();
        assert_eq!(
            sw.installed_version().unwrap(),
            Version::parse("0.7.0").unwrap()
        );
    }

    #[test]
    fn a_failing_installer_is_an_error_and_the_installed_version_is_untouched() {
        let (sw, _r, _i, dir) = rig(1603);
        let art = installer(&dir, "bad-setup.exe", "0.7.0");
        let e = sw.switch(&art).unwrap_err();
        assert!(e.to_string().contains("1603"), "{e}");
        assert_eq!(
            sw.installed_version().unwrap(),
            Version::parse("0.6.0").unwrap()
        );
    }

    #[test]
    fn rollback_reruns_the_retained_installer_of_the_previous_version() {
        let (sw, _r, _i, dir) = rig(0);
        let v6 = installer(&dir, "v6-setup.exe", "0.6.0");
        let v7 = installer(&dir, "v7-setup.exe", "0.7.0");
        sw.retain(&v6, &Version::parse("0.6.0").unwrap()).unwrap();
        sw.switch(&v7).unwrap();
        assert_eq!(
            sw.installed_version().unwrap(),
            Version::parse("0.7.0").unwrap()
        );
        sw.restore_previous(&RollbackInfo {
            previous_version: Version::parse("0.6.0").unwrap(),
            failed_version: Version::parse("0.7.0").unwrap(),
        })
        .unwrap();
        assert_eq!(
            sw.installed_version().unwrap(),
            Version::parse("0.6.0").unwrap()
        );
    }

    #[test]
    fn rollback_without_a_retained_installer_is_refused_loudly_not_faked() {
        let (sw, _r, _i, _d) = rig(0);
        let e = sw
            .restore_previous(&RollbackInfo {
                previous_version: Version::parse("0.5.0").unwrap(),
                failed_version: Version::parse("0.6.0").unwrap(),
            })
            .unwrap_err();
        assert!(e.to_string().contains("no retained installer"), "{e}");
    }

    #[test]
    fn detached_mode_launches_and_returns_without_waiting() {
        let (sw, r, _i, dir) = rig(0);
        let sw = sw.detached(true);
        let art = installer(&dir, "d-setup.exe", "0.7.0");
        sw.switch(&art).unwrap();
        assert_eq!(*r.detached_calls.lock().unwrap(), 1);
        // nada instalou ainda: a confirmação é da próxima partida (recover + marcador de saúde)
        assert_eq!(
            sw.installed_version().unwrap(),
            Version::parse("0.6.0").unwrap()
        );
    }

    #[test]
    fn retained_names_cannot_escape_the_directory() {
        let v = Version::parse("1.0.0-rc.1+build.5").unwrap();
        let n = retained_name(&v);
        assert!(
            !n.contains('/') && !n.contains('\\') && n.ends_with("-setup.exe"),
            "{n}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_process_runner_runs_a_real_program_without_a_shell_and_honors_the_timeout() {
        let r = ProcessRunner;
        assert_eq!(
            r.run_wait(Path::new("/bin/true"), &[], Duration::from_secs(5))
                .unwrap(),
            0
        );
        assert_ne!(
            r.run_wait(Path::new("/bin/false"), &[], Duration::from_secs(5))
                .unwrap(),
            0
        );
        // argumentos nunca passam por um shell: `;` é só um argumento
        assert_eq!(
            r.run_wait(
                Path::new("/bin/echo"),
                &["a; rm -rf /".to_owned()],
                Duration::from_secs(5)
            )
            .unwrap(),
            0
        );
        let e = r
            .run_wait(
                Path::new("/bin/sleep"),
                &["5".to_owned()],
                Duration::from_millis(300),
            )
            .unwrap_err();
        assert!(e.to_string().contains("timed out"), "{e}");
        assert!(
            r.run_wait(
                Path::new("/definitely/not/here"),
                &[],
                Duration::from_secs(1)
            )
            .is_err()
        );
    }
}
