//! Execução de processos externos com **teto de saída** e **timeout** (ADR-047 §3). Sem shell:
//! programa e argumentos são passados estruturados; stdin fechado.

use crate::error::{MediaError, MediaErrorCode};
use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct RunLimits {
    pub timeout: Duration,
    pub max_stdout: usize,
    pub max_stderr: usize,
}

#[derive(Debug)]
pub struct RunOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Leitor limitado: acumula até `cap` bytes num buffer compartilhado. Nunca é *joined* depois de um
/// kill (um neto do processo pode manter o pipe aberto): o pai só espera `done` por um tempo curto.
struct Reader {
    buf: Arc<Mutex<Vec<u8>>>,
    done: Arc<AtomicBool>,
}

fn drain<R: Read + Send + 'static>(mut r: R, cap: usize, exceeded: Arc<AtomicBool>) -> Reader {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let done = Arc::new(AtomicBool::new(false));
    let (b2, d2) = (Arc::clone(&buf), Arc::clone(&done));
    std::thread::spawn(move || {
        let mut chunk = [0u8; 16 * 1024];
        loop {
            match r.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let Ok(mut out) = b2.lock() else { break };
                    if out.len().saturating_add(n) > cap {
                        let room = cap.saturating_sub(out.len());
                        out.extend_from_slice(&chunk[..room]);
                        exceeded.store(true, Ordering::SeqCst);
                        break;
                    }
                    out.extend_from_slice(&chunk[..n]);
                }
            }
        }
        d2.store(true, Ordering::SeqCst);
    });
    Reader { buf, done }
}

impl Reader {
    /// Espera o fim da leitura por no máximo `grace` e devolve o que foi lido.
    fn take(&self, grace: Duration) -> Vec<u8> {
        let t = Instant::now();
        while !self.done.load(Ordering::SeqCst) && t.elapsed() < grace {
            std::thread::sleep(Duration::from_millis(2));
        }
        self.buf
            .lock()
            .map(|mut b| std::mem::take(&mut *b))
            .unwrap_or_default()
    }
}

/// Executa `program args…`; mata o processo ao estourar o timeout ou o teto de saída.
pub fn run_bounded(
    program: &Path,
    args: &[OsString],
    limits: &RunLimits,
) -> Result<RunOutput, MediaError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    // ETXTBSY: o executável acabou de ser escrito/atualizado e algum fork concorrente ainda segura o
    // descritor de escrita; é transitório — tenta de novo por um instante.
    let mut attempt = 0;
    let spawned = loop {
        match cmd.spawn() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 50 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            other => break other,
        }
    };
    let mut child = spawned.map_err(|e| {
        let code = if e.kind() == std::io::ErrorKind::NotFound {
            MediaErrorCode::MediaBackendNotFound
        } else {
            MediaErrorCode::MediaBackendFailed
        };
        MediaError::new(code, format!("cannot start `{}`: {e}", program.display()))
    })?;
    let exceeded = Arc::new(AtomicBool::new(false));
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(MediaError::new(
            MediaErrorCode::MediaBackendFailed,
            "cannot capture the process output",
        ));
    };
    let out_t = drain(out, limits.max_stdout, Arc::clone(&exceeded));
    let err_t = drain(err, limits.max_stderr, Arc::clone(&exceeded));

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(e) => break Err(format!("wait failed: {e}")),
        }
        if exceeded.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(MediaError::new(
                MediaErrorCode::MediaProbeOutputTooLarge,
                format!("`{}` produced more output than allowed", program.display()),
            ));
        }
        if started.elapsed() >= limits.timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(MediaError::new(
                MediaErrorCode::MediaProbeTimeout,
                format!(
                    "`{}` did not finish within {} ms and was killed",
                    program.display(),
                    limits.timeout.as_millis()
                ),
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let status = status.map_err(|m| {
        let _ = child.kill();
        MediaError::new(MediaErrorCode::MediaBackendFailed, m)
    })?;
    let stdout = out_t.take(Duration::from_secs(2));
    let stderr = err_t.take(Duration::from_secs(2));
    if exceeded.load(Ordering::SeqCst) {
        return Err(MediaError::new(
            MediaErrorCode::MediaProbeOutputTooLarge,
            format!("`{}` produced more output than allowed", program.display()),
        ));
    }
    Ok(RunOutput {
        status,
        stdout,
        stderr,
    })
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn lim(ms: u64, out: usize) -> RunLimits {
        RunLimits {
            timeout: Duration::from_millis(ms),
            max_stdout: out,
            max_stderr: 1024,
        }
    }
    fn sh(script: &str) -> (std::path::PathBuf, Vec<OsString>) {
        ("/bin/sh".into(), vec!["-c".into(), script.into()])
    }

    #[test]
    fn captures_output_and_status() {
        let (p, a) = sh("printf hello; printf oops >&2; exit 3");
        let o = run_bounded(&p, &a, &lim(5000, 1024)).unwrap();
        assert_eq!(o.stdout, b"hello");
        assert_eq!(o.stderr, b"oops");
        assert_eq!(o.status.code(), Some(3));
    }

    #[test]
    fn a_hung_process_is_killed_at_the_timeout() {
        let (p, a) = sh("sleep 30");
        let t = Instant::now();
        let e = run_bounded(&p, &a, &lim(200, 1024)).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaProbeTimeout);
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "was not killed promptly"
        );
    }

    #[test]
    fn runaway_stdout_is_capped_and_the_process_killed() {
        let (p, a) = sh("yes AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
        let t = Instant::now();
        let e = run_bounded(&p, &a, &lim(20_000, 64 * 1024)).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaProbeOutputTooLarge);
        assert!(t.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn runaway_stderr_is_capped_too() {
        let (p, a) = sh("yes AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA >&2");
        let t = Instant::now();
        let e = run_bounded(&p, &a, &lim(20_000, 1024)).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaProbeOutputTooLarge);
        assert!(t.elapsed() < Duration::from_secs(10));
    }

    /// O filho é MORTO de verdade no timeout (não só abandonado): o pid deixa de existir.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_child_is_really_gone_after_a_timeout() {
        let pidfile = std::env::temp_dir().join(format!("capia-pid-{}", std::process::id()));
        let _ = std::fs::remove_file(&pidfile);
        let (p, a) = sh(&format!("echo $$ > '{}'; exec sleep 30", pidfile.display()));
        let e = run_bounded(&p, &a, &lim(600, 1024)).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaProbeTimeout);
        let pid: u32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "process {pid} survived the timeout"
        );
        let _ = std::fs::remove_file(&pidfile);
    }

    #[test]
    fn missing_program_is_backend_not_found() {
        let e = run_bounded(Path::new("/nonexistent/ffprobe"), &[], &lim(1000, 10)).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaBackendNotFound);
    }

    #[test]
    fn arguments_are_never_interpreted_by_a_shell() {
        // metacaracteres chegam literais ao programa
        let p = std::path::PathBuf::from("/bin/echo");
        let o = run_bounded(
            &p,
            &["$(touch /tmp/capia-pwn); `id` | ; &&".into()],
            &lim(5000, 1024),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&o.stdout).trim(),
            "$(touch /tmp/capia-pwn); `id` | ; &&"
        );
        assert!(!Path::new("/tmp/capia-pwn").exists());
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn lim(ms: u64, out: usize) -> RunLimits {
        RunLimits {
            timeout: Duration::from_millis(ms),
            max_stdout: out,
            max_stderr: 1024,
        }
    }

    fn cmd(script: &str) -> (std::path::PathBuf, Vec<OsString>) {
        (
            "cmd.exe".into(),
            vec!["/d".into(), "/c".into(), script.into()],
        )
    }

    #[test]
    fn captures_output_and_status() {
        let (p, a) = cmd("echo hello& exit 3");
        let o = run_bounded(&p, &a, &lim(10_000, 1024)).unwrap();
        assert!(String::from_utf8_lossy(&o.stdout).contains("hello"));
        assert_eq!(o.status.code(), Some(3));
    }

    #[test]
    fn a_hung_process_is_killed_at_the_timeout() {
        // `ping -n 30` segura ~30 s
        let (p, a) = cmd("ping -n 30 127.0.0.1 > nul");
        let t = Instant::now();
        let e = run_bounded(&p, &a, &lim(500, 1024)).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaProbeTimeout);
        assert!(
            t.elapsed() < Duration::from_secs(15),
            "was not killed promptly"
        );
    }

    #[test]
    fn runaway_stdout_is_capped_and_the_process_killed() {
        let (p, a) = cmd("for /l %i in (1,1,100000000) do @echo AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
        let t = Instant::now();
        let e = run_bounded(&p, &a, &lim(60_000, 64 * 1024)).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaProbeOutputTooLarge);
        assert!(t.elapsed() < Duration::from_secs(30));
    }

    #[test]
    fn missing_program_is_backend_not_found() {
        let e =
            run_bounded(Path::new("C:\\no\\such\\ffprobe.exe"), &[], &lim(1000, 10)).unwrap_err();
        assert_eq!(e.code, MediaErrorCode::MediaBackendNotFound);
    }
}
