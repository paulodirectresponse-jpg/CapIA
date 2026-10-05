//! Política de crash report (Fase 6 §24, ADR-draft "crash reporting opt-in").
//!
//! * O gancho de pânico **sempre** grava um registro **local** redigido (nunca sai do computador sozinho).
//! * O envio é **opt-in** (desligado por padrão) e só acontece **na próxima abertura** (`flush_pending`), nunca
//!   dentro do pânico; e só por um [`CrashSink`] configurado. Este crate **não tem código de rede**: o destino de
//!   upload é uma pendência externa (`UnconfiguredSink` ⇒ nada é enviado).
//! * Conteúdo do registro: versão, SO/arquitetura, id do build, local do pânico (arquivo:linha), mensagem **redigida**.
//!   Nunca: chaves, mídia, prompts, conteúdo de projeto, caminhos pessoais, *backtrace*.

use crate::buildinfo::BuildInfo;
use crate::error::SupportError;
use crate::util::{now_ms, random_hex, sanitize, truncate_chars};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) const MAX_MESSAGE_BYTES: usize = 1024;
pub const DEFAULT_MAX_RECORDS: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashRecord {
    pub schema: u32,
    pub id: String,
    pub ts_ms: u64,
    pub app_version: String,
    pub os: String,
    pub arch: String,
    pub build_id: Option<String>,
    /// `arquivo:linha:coluna` (só os 3 últimos componentes do caminho).
    pub location: Option<String>,
    pub message: String,
    pub thread: Option<String>,
    /// `true` depois que um sink confirmou o recebimento (só com opt-in).
    pub uploaded: bool,
}

/// Mantém só os 3 últimos componentes do caminho (sem diretório pessoal) e sanitiza.
fn short_location(file: &str, line: u32, col: u32) -> String {
    let parts: Vec<&str> = file.split(['/', '\\']).collect();
    let tail = parts[parts.len().saturating_sub(3)..].join("/");
    sanitize(&format!("{tail}:{line}:{col}"))
}

impl CrashRecord {
    pub fn new(
        build: &BuildInfo,
        message: &str,
        location: Option<(&str, u32, u32)>,
        thread: Option<&str>,
    ) -> Self {
        Self {
            schema: 1,
            id: random_hex(),
            ts_ms: now_ms(),
            app_version: build.version.clone(),
            os: build.os.clone(),
            arch: build.arch.clone(),
            build_id: build.build_id.clone(),
            location: location.map(|(f, l, c)| short_location(f, l, c)),
            message: truncate_chars(&sanitize(message), MAX_MESSAGE_BYTES),
            thread: thread.map(sanitize),
            uploaded: false,
        }
    }
}

/// Destino de envio. A implementação real (HTTPS para o endpoint do produto) é externa a este crate.
pub trait CrashSink: Send + Sync + std::fmt::Debug {
    fn submit(&self, record: &CrashRecord) -> Result<(), SupportError>;
}

/// Sink padrão: endpoint não configurado ⇒ nada sai da máquina.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnconfiguredSink;

impl CrashSink for UnconfiguredSink {
    fn submit(&self, _record: &CrashRecord) -> Result<(), SupportError> {
        Err(SupportError::SinkNotConfigured)
    }
}

/// Registros locais (`crash-<ms>-<id>.json`), limitados em quantidade.
#[derive(Debug, Clone)]
pub struct CrashStore {
    dir: PathBuf,
    max_records: usize,
}

impl CrashStore {
    pub fn new(dir: &Path, max_records: usize) -> Result<Self, SupportError> {
        fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            max_records: max_records.max(1),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file_of(&self, r: &CrashRecord) -> PathBuf {
        self.dir
            .join(format!("crash-{:013}-{}.json", r.ts_ms, r.id))
    }

    pub fn write(&self, r: &CrashRecord) -> Result<PathBuf, SupportError> {
        let path = self.file_of(r);
        let json =
            serde_json::to_vec_pretty(r).map_err(|e| SupportError::Invalid(e.to_string()))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &path)?;
        self.prune();
        Ok(path)
    }

    fn files(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".json"))
            })
            .collect();
        v.sort();
        v
    }

    fn prune(&self) {
        let files = self.files();
        let excess = files.len().saturating_sub(self.max_records);
        for f in files.into_iter().take(excess) {
            let _ = fs::remove_file(f);
        }
    }

    /// Registros legíveis, do mais antigo ao mais novo. Arquivos ilegíveis são ignorados.
    pub fn list(&self) -> Vec<(PathBuf, CrashRecord)> {
        self.files()
            .into_iter()
            .filter_map(|p| {
                let r: CrashRecord = serde_json::from_slice(&fs::read(&p).ok()?).ok()?;
                Some((p, r))
            })
            .collect()
    }

    fn mark_uploaded(&self, path: &Path, r: &CrashRecord) -> Result<(), SupportError> {
        let mut r = r.clone();
        r.uploaded = true;
        let json =
            serde_json::to_vec_pretty(&r).map_err(|e| SupportError::Invalid(e.to_string()))?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Apaga todos os registros locais (ação do usuário).
    pub fn clear(&self) -> usize {
        let files = self.files();
        let n = files.len();
        for f in files {
            let _ = fs::remove_file(f);
        }
        n
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CrashStatus {
    pub opt_in: bool,
    pub sink_configured: bool,
    pub local_records: usize,
    pub pending_upload: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FlushReport {
    pub attempted: usize,
    pub sent: usize,
    /// Por que nada foi tentado (`opted_out`, `no_records`), se for o caso.
    pub skipped: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub struct CrashReporter {
    store: CrashStore,
    build: BuildInfo,
    opt_in: Arc<AtomicBool>,
    sink: Arc<dyn CrashSink>,
    sink_configured: bool,
}

impl CrashReporter {
    /// `opt_in` é o flag de `SettingsStore::opt_in_flag` (padrão `false`).
    pub fn new(
        store: CrashStore,
        build: BuildInfo,
        opt_in: Arc<AtomicBool>,
        sink: Option<Arc<dyn CrashSink>>,
    ) -> Self {
        let sink_configured = sink.is_some();
        Self {
            store,
            build,
            opt_in,
            sink: sink.unwrap_or_else(|| Arc::new(UnconfiguredSink)),
            sink_configured,
        }
    }

    pub fn store(&self) -> &CrashStore {
        &self.store
    }

    pub fn status(&self) -> CrashStatus {
        let list = self.store.list();
        CrashStatus {
            opt_in: self.opt_in.load(Ordering::SeqCst),
            sink_configured: self.sink_configured,
            local_records: list.len(),
            pending_upload: list.iter().filter(|(_, r)| !r.uploaded).count(),
        }
    }

    /// Grava o registro local de um pânico (sempre; independe de opt-in). Nunca entra em pânico.
    pub fn record_panic(
        &self,
        message: &str,
        location: Option<(&str, u32, u32)>,
        thread: Option<&str>,
    ) -> Option<PathBuf> {
        let r = CrashRecord::new(&self.build, message, location, thread);
        self.store.write(&r).ok()
    }

    /// Instala o gancho de pânico: stderr redigido + registro local. **Não envia nada.**
    pub fn install_panic_hook(self: &Arc<Self>) {
        let me = Arc::clone(self);
        std::panic::set_hook(Box::new(move |info| {
            let payload = info.payload();
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic with a non-string payload".to_owned());
            let loc = info.location().map(|l| (l.file(), l.line(), l.column()));
            let thread = std::thread::current().name().map(str::to_owned);
            eprintln!("{}", sanitize(&info.to_string()));
            let _ = me.record_panic(&msg, loc, thread.as_deref());
        }));
    }

    /// Envia os registros pendentes **somente** com opt-in e sink configurado (chamar na abertura do app).
    pub fn flush_pending(&self) -> FlushReport {
        if !self.opt_in.load(Ordering::SeqCst) {
            return FlushReport {
                attempted: 0,
                sent: 0,
                skipped: Some("opted_out"),
            };
        }
        let pending: Vec<_> = self
            .store
            .list()
            .into_iter()
            .filter(|(_, r)| !r.uploaded)
            .collect();
        if pending.is_empty() {
            return FlushReport {
                attempted: 0,
                sent: 0,
                skipped: Some("no_records"),
            };
        }
        let (mut attempted, mut sent) = (0, 0);
        for (path, rec) in pending {
            attempted += 1;
            // reconfere o opt-in a cada envio: o usuário pode desligar durante o flush
            if !self.opt_in.load(Ordering::SeqCst) {
                break;
            }
            if self.sink.submit(&rec).is_ok() {
                let _ = self.store.mark_uploaded(&path, &rec);
                sent += 1;
            }
        }
        FlushReport {
            attempted,
            sent,
            skipped: None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::util::testutil::Tmp;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct CountingSink {
        got: Mutex<Vec<CrashRecord>>,
    }
    impl CrashSink for CountingSink {
        fn submit(&self, r: &CrashRecord) -> Result<(), SupportError> {
            self.got.lock().unwrap().push(r.clone());
            Ok(())
        }
    }

    fn reporter(t: &Tmp, opt: bool, sink: Option<Arc<dyn CrashSink>>) -> Arc<CrashReporter> {
        Arc::new(CrashReporter::new(
            CrashStore::new(&t.0, 3).unwrap(),
            BuildInfo::current(),
            Arc::new(AtomicBool::new(opt)),
            sink,
        ))
    }

    #[test]
    fn local_record_is_redacted_and_minimal() {
        const CANARY: &str = "CNRY-crash-0f3a91c47e2db865";
        capia_secrets::register_global(CANARY);
        let t = Tmp::new("crash");
        let r = reporter(&t, false, None);
        let p = r
            .record_panic(
                &format!("boom with {CANARY} and Authorization: Bearer abcdefghijklmnop"),
                Some(("/home/someone/secret-project/crates/x/src/lib.rs", 10, 5)),
                Some("worker"),
            )
            .unwrap();
        let text = fs::read_to_string(p).unwrap();
        assert!(!text.contains(CANARY));
        assert!(!text.contains("abcdefghijklmnop"));
        assert!(
            !text.contains("secret-project"),
            "only the last 3 path components: {text}"
        );
        assert!(text.contains("x/src/lib.rs:10:5"));
        let rec: CrashRecord = serde_json::from_str(&text).unwrap();
        assert_eq!(rec.app_version, env!("CARGO_PKG_VERSION"));
        assert!(!rec.uploaded);
    }

    #[test]
    fn nothing_is_sent_without_opt_in_even_with_a_sink() {
        let t = Tmp::new("crash2");
        let sink = Arc::new(CountingSink::default());
        let r = reporter(&t, false, Some(sink.clone()));
        r.record_panic("x", None, None).unwrap();
        let rep = r.flush_pending();
        assert_eq!(rep.skipped, Some("opted_out"));
        assert!(sink.got.lock().unwrap().is_empty());
        assert_eq!(r.status().pending_upload, 1);
    }

    #[test]
    fn opt_in_sends_once_and_marks_uploaded_and_opt_out_stops_it() {
        let t = Tmp::new("crash3");
        let sink = Arc::new(CountingSink::default());
        let opt = Arc::new(AtomicBool::new(true));
        let r = Arc::new(CrashReporter::new(
            CrashStore::new(&t.0, 10).unwrap(),
            BuildInfo::current(),
            opt.clone(),
            Some(sink.clone()),
        ));
        r.record_panic("one", None, None).unwrap();
        let rep = r.flush_pending();
        assert_eq!((rep.attempted, rep.sent), (1, 1));
        // idempotente
        assert_eq!(r.flush_pending().skipped, Some("no_records"));
        assert_eq!(sink.got.lock().unwrap().len(), 1);
        // desligar interrompe novos envios
        r.record_panic("two", None, None).unwrap();
        opt.store(false, Ordering::SeqCst);
        assert_eq!(r.flush_pending().skipped, Some("opted_out"));
        assert_eq!(sink.got.lock().unwrap().len(), 1);
    }

    #[test]
    fn unconfigured_sink_keeps_records_local() {
        let t = Tmp::new("crash4");
        let r = reporter(&t, true, None);
        r.record_panic("x", None, None).unwrap();
        let rep = r.flush_pending();
        assert_eq!((rep.attempted, rep.sent), (1, 0));
        let st = r.status();
        assert!(!st.sink_configured && st.pending_upload == 1);
    }

    #[test]
    fn local_records_are_bounded() {
        let t = Tmp::new("crash5");
        let r = reporter(&t, false, None);
        for i in 0..10 {
            r.record_panic(&format!("p{i}"), None, None).unwrap();
        }
        let n = r.store().list().len();
        assert!((1..=3).contains(&n));
        assert_eq!(r.store().clear(), n);
        assert!(r.store().list().is_empty());
    }

    #[test]
    fn panic_hook_writes_a_local_record_and_uploads_nothing() {
        const CANARY: &str = "CNRY-hook-9b8c7d6e5f4a3b2c";
        capia_secrets::register_global(CANARY);
        let t = Tmp::new("crash6");
        let sink = Arc::new(CountingSink::default());
        let r = reporter(&t, false, Some(sink.clone()));
        let prev = std::panic::take_hook();
        r.install_panic_hook();
        let res = std::panic::catch_unwind(|| panic!("provider failed: key {CANARY}"));
        std::panic::set_hook(prev);
        assert!(res.is_err());
        let list = r.store().list();
        assert_eq!(list.len(), 1);
        assert!(!list[0].1.message.contains(CANARY));
        assert!(
            list[0]
                .1
                .location
                .as_deref()
                .is_some_and(|l| l.contains("crash.rs"))
        );
        assert!(
            sink.got.lock().unwrap().is_empty(),
            "the hook must never upload"
        );
    }
}
