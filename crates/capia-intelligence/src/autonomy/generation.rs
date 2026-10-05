//! Geração de mídia (PHASE5_MEMORY_GATEWAY §22..§29, §40): imagem/vídeo/TTS como **jobs** de
//! providers de geração (separados do provider do Brain). Tudo é opcional: desligado ⇒ o Planner
//! vê a capability indisponível e o app segue funcionando.
//!
//! Idempotência (geração externa é cara): `idempotency_key` determinística; o provider devolve
//! `job_id`; o orquestrador **persiste** `job_id` no livro de efeitos e, depois de um crash, só
//! *consulta* (`poll`/`lookup`) — nunca submete de novo sem confirmar que o job anterior não existe.
//! A mídia gerada vira asset normal (staging → hash → proveniência → import) e versões novas nunca
//! sobrescrevem o arquivo anterior.

use async_trait::async_trait;
use capia_ai::CancelToken;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenKind {
    Image,
    Video,
    Tts,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenRequest {
    pub kind: GenKind,
    pub purpose: String,
    /// Prompt/especificação (hash e referência vão para a proveniência; o texto também).
    pub prompt: String,
    pub reference_assets: Vec<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<i64>,
    pub model: String,
    pub safety: Vec<String>,
    /// Determinística por (run, need, prompt, params): o mesmo pedido ⇒ a mesma chave.
    pub idempotency_key: String,
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl GenError {
    pub fn new(code: &str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.to_owned(),
            message: capia_secrets::redact_registered_global(&message.into()),
            retryable,
        }
    }
}

impl core::fmt::Display for GenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for GenError {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GenStatus {
    Pending,
    Running,
    /// Pronto: o provider escreveu o arquivo em `path` (staging).
    Done { path: PathBuf, content_type: Option<String> },
    Failed { message: String, retryable: bool },
}

#[async_trait]
pub trait GenerationProvider: Send + Sync {
    fn id(&self) -> String;
    fn kinds(&self) -> Vec<GenKind>;
    /// Estimativa em micros (`None` = preço desconhecido).
    fn estimate(&self, req: &GenRequest) -> Option<u64>;
    /// Procura um job já existente para a chave (para retomar depois de crash).
    async fn lookup(&self, idempotency_key: &str) -> Result<Option<String>, GenError>;
    /// Submete (idempotente na própria chave quando o provider suportar). Devolve o `job_id`.
    async fn submit(&self, req: &GenRequest) -> Result<String, GenError>;
    async fn poll(&self, job_id: &str, staging: &Path, cancel: &CancelToken) -> Result<GenStatus, GenError>;
    async fn cancel(&self, job_id: &str) -> Result<(), GenError>;
}

impl core::fmt::Debug for dyn GenerationProvider {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "GenerationProvider({})", self.id())
    }
}

#[derive(Default)]
pub struct GenerationRegistry {
    providers: Vec<Arc<dyn GenerationProvider>>,
    enabled: Mutex<bool>,
}

impl core::fmt::Debug for GenerationRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GenerationRegistry")
            .field("providers", &self.providers.iter().map(|p| p.id()).collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl GenerationRegistry {
    pub fn new() -> Self {
        Self { providers: Vec::new(), enabled: Mutex::new(true) }
    }

    pub fn register(&mut self, p: Arc<dyn GenerationProvider>) {
        self.providers.retain(|x| x.id() != p.id());
        self.providers.push(p);
    }

    pub fn set_enabled(&self, on: bool) {
        if let Ok(mut e) = self.enabled.lock() {
            *e = on;
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.lock().is_ok_and(|e| *e)
    }

    /// Provider ativo que serve o tipo (o primeiro registrado).
    pub fn for_kind(&self, k: GenKind) -> Option<Arc<dyn GenerationProvider>> {
        if !self.is_enabled() {
            return None;
        }
        self.providers.iter().find(|p| p.kinds().contains(&k)).cloned()
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn GenerationProvider>> {
        self.providers.iter().find(|p| p.id() == id).cloned()
    }

    pub fn available(&self) -> bool {
        self.is_enabled() && !self.providers.is_empty()
    }
}

/// Provider Replay (determinístico, sem rede) — contador de submits por chave para provar que um
/// resume depois de crash **não** gera de novo.
pub struct ReplayGenerationProvider {
    id: String,
    kinds: Vec<GenKind>,
    price_micros: Option<u64>,
    payload: Vec<u8>,
    /// `poll` devolve `Running` esta quantidade de vezes antes de concluir.
    running_polls: u32,
    jobs: Mutex<BTreeMap<String, JobState>>,
    pub submits: AtomicU32,
    pub polls: AtomicU32,
    fail_submit: Mutex<Option<GenError>>,
    invalid_output: bool,
}

#[derive(Clone, Debug)]
struct JobState {
    key: String,
    polls_left: u32,
}

impl core::fmt::Debug for ReplayGenerationProvider {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReplayGenerationProvider").field("id", &self.id).finish_non_exhaustive()
    }
}

impl ReplayGenerationProvider {
    pub fn new(id: &str, kinds: Vec<GenKind>, price_micros: Option<u64>, payload: Vec<u8>) -> Self {
        Self {
            id: id.to_owned(),
            kinds,
            price_micros,
            payload,
            running_polls: 1,
            jobs: Mutex::new(BTreeMap::new()),
            submits: AtomicU32::new(0),
            polls: AtomicU32::new(0),
            fail_submit: Mutex::new(None),
            invalid_output: false,
        }
    }

    pub fn with_running_polls(mut self, n: u32) -> Self {
        self.running_polls = n;
        self
    }

    pub fn producing_empty_output(mut self) -> Self {
        self.invalid_output = true;
        self
    }

    pub fn fail_next_submit(&self, e: GenError) {
        if let Ok(mut g) = self.fail_submit.lock() {
            *g = Some(e);
        }
    }

    pub fn submit_count(&self) -> u32 {
        self.submits.load(Ordering::SeqCst)
    }

    /// Simula "o job existe no provider" para um novo processo (o job vive no provider, não no app).
    pub fn job_count(&self) -> usize {
        self.jobs.lock().map_or(0, |j| j.len())
    }
}

#[async_trait]
impl GenerationProvider for ReplayGenerationProvider {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn kinds(&self) -> Vec<GenKind> {
        self.kinds.clone()
    }

    fn estimate(&self, _req: &GenRequest) -> Option<u64> {
        self.price_micros
    }

    async fn lookup(&self, key: &str) -> Result<Option<String>, GenError> {
        Ok(self
            .jobs
            .lock()
            .ok()
            .and_then(|j| j.iter().find(|(_, s)| s.key == key).map(|(id, _)| id.clone())))
    }

    async fn submit(&self, req: &GenRequest) -> Result<String, GenError> {
        if let Some(e) = self.fail_submit.lock().ok().and_then(|mut g| g.take()) {
            return Err(e);
        }
        // idempotente na chave: o mesmo pedido devolve o mesmo job (sem cobrar de novo)
        if let Some(existing) = self.lookup(&req.idempotency_key).await? {
            return Ok(existing);
        }
        let n = self.submits.fetch_add(1, Ordering::SeqCst) + 1;
        let job = format!("job-{}-{n}", self.id);
        if let Ok(mut j) = self.jobs.lock() {
            j.insert(job.clone(), JobState { key: req.idempotency_key.clone(), polls_left: self.running_polls });
        }
        Ok(job)
    }

    async fn poll(&self, job_id: &str, staging: &Path, cancel: &CancelToken) -> Result<GenStatus, GenError> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        if cancel.is_cancelled() {
            return Err(GenError::new("CANCELLED", "cancelled", false));
        }
        let left = {
            let mut j = self.jobs.lock().map_err(|_| GenError::new("INTERNAL", "poisoned", false))?;
            let s = j.get_mut(job_id).ok_or_else(|| GenError::new("NOT_FOUND", "unknown job", false))?;
            if s.polls_left > 0 {
                s.polls_left -= 1;
                return Ok(if s.polls_left == 0 { GenStatus::Running } else { GenStatus::Pending });
            }
            s.polls_left
        };
        let _ = left;
        if let Some(dir) = staging.parent() {
            std::fs::create_dir_all(dir).map_err(|e| GenError::new("IO", e.to_string(), false))?;
        }
        let part = staging.with_extension("part");
        let bytes: &[u8] = if self.invalid_output { &[] } else { &self.payload };
        std::fs::write(&part, bytes).map_err(|e| GenError::new("IO", e.to_string(), false))?;
        std::fs::rename(&part, staging).map_err(|e| GenError::new("IO", e.to_string(), false))?;
        Ok(GenStatus::Done { path: staging.to_path_buf(), content_type: Some("video/mp4".into()) })
    }

    async fn cancel(&self, job_id: &str) -> Result<(), GenError> {
        if let Ok(mut j) = self.jobs.lock() {
            j.remove(job_id);
        }
        Ok(())
    }
}

/// Chave de idempotência determinística de um pedido de geração.
pub fn idempotency_key(run_id: &str, need_id: &str, prompt: &str, model: &str, version: u32) -> String {
    format!(
        "gen:{run_id}:{need_id}:v{version}:{}",
        super::plan::digest_value(&serde_json::json!([prompt, model]))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(key: &str) -> GenRequest {
        GenRequest {
            kind: GenKind::Video,
            purpose: "b-roll".into(),
            prompt: "a person cooking".into(),
            reference_assets: vec![],
            width: Some(720),
            height: Some(1280),
            duration_ms: Some(4000),
            model: "m1".into(),
            safety: vec![],
            idempotency_key: key.into(),
            params: Value::Null,
        }
    }

    #[tokio::test]
    async fn the_same_key_never_creates_a_second_job() {
        let p = ReplayGenerationProvider::new("g", vec![GenKind::Video], Some(500_000), b"video".to_vec());
        let a = p.submit(&req("k1")).await.unwrap();
        let b = p.submit(&req("k1")).await.unwrap();
        assert_eq!(a, b);
        assert_eq!(p.submit_count(), 1);
        assert_eq!(p.lookup("k1").await.unwrap(), Some(a));
        assert_eq!(p.lookup("other").await.unwrap(), None);
        let c = p.submit(&req("k2")).await.unwrap();
        assert_ne!(c, b);
        assert_eq!(p.submit_count(), 2);
    }

    #[tokio::test]
    async fn polling_reaches_done_and_writes_a_staging_file() {
        let d = std::env::temp_dir().join(format!("capia-gen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = ReplayGenerationProvider::new("g", vec![GenKind::Video], None, b"video".to_vec()).with_running_polls(2);
        let job = p.submit(&req("k")).await.unwrap();
        let cancel = CancelToken::new();
        let out = d.join("out.mp4");
        let mut statuses = Vec::new();
        for _ in 0..5 {
            let s = p.poll(&job, &out, &cancel).await.unwrap();
            let done = matches!(s, GenStatus::Done { .. });
            statuses.push(s);
            if done {
                break;
            }
        }
        assert!(matches!(statuses.last(), Some(GenStatus::Done { .. })));
        assert!(statuses.len() >= 3, "pending/running before done: {statuses:?}");
        assert_eq!(std::fs::read(&out).unwrap(), b"video");
        assert_eq!(p.estimate(&req("k")), None, "unknown price stays unknown");
    }

    #[test]
    fn the_registry_hides_providers_when_generation_is_off() {
        let mut r = GenerationRegistry::new();
        assert!(!r.available());
        r.register(Arc::new(ReplayGenerationProvider::new("g", vec![GenKind::Video], Some(1), vec![1])));
        assert!(r.for_kind(GenKind::Video).is_some());
        assert!(r.for_kind(GenKind::Tts).is_none());
        r.set_enabled(false);
        assert!(r.for_kind(GenKind::Video).is_none() && !r.available());
    }

    #[test]
    fn idempotency_keys_depend_on_run_need_prompt_model_and_version() {
        let a = idempotency_key("r", "n", "p", "m", 1);
        assert_eq!(a, idempotency_key("r", "n", "p", "m", 1));
        for b in [
            idempotency_key("r2", "n", "p", "m", 1),
            idempotency_key("r", "n2", "p", "m", 1),
            idempotency_key("r", "n", "p2", "m", 1),
            idempotency_key("r", "n", "p", "m2", 1),
            idempotency_key("r", "n", "p", "m", 2),
        ] {
            assert_ne!(a, b);
        }
    }
}
