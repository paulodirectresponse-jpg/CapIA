use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub const CODE_CANCELLED: &str = "JOB_CANCELLED";
pub const CODE_PANICKED: &str = "JOB_PANICKED";

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub String);

impl core::fmt::Display for JobId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    AssetHash,
    FrameIndex,
    Waveform,
    AudioIndex,
    Proxy,
    Thumbnail,
    BatchRelink,
    /// Extensão sem mudar o executor (testes, futuras operações).
    Custom(String),
}

impl JobKind {
    pub fn as_str(&self) -> String {
        match self {
            Self::AssetHash => "asset_hash".into(),
            Self::FrameIndex => "frame_index".into(),
            Self::Waveform => "waveform".into(),
            Self::AudioIndex => "audio_index".into(),
            Self::Proxy => "proxy".into(),
            Self::Thumbnail => "thumbnail".into(),
            Self::BatchRelink => "batch_relink".into(),
            Self::Custom(s) => format!("custom:{s}"),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "asset_hash" => Self::AssetHash,
            "frame_index" => Self::FrameIndex,
            "waveform" => Self::Waveform,
            "audio_index" => Self::AudioIndex,
            "proxy" => Self::Proxy,
            "thumbnail" => Self::Thumbnail,
            "batch_relink" => Self::BatchRelink,
            other => Self::Custom(other.strip_prefix("custom:")?.to_owned()),
        })
    }
}

/// Categoria de prioridade (índice = ordem de preferência).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Interactive,
    Normal,
    Background,
}

impl Priority {
    pub const ALL: [Priority; 3] = [Self::Interactive, Self::Normal, Self::Background];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Normal => "normal",
            Self::Background => "background",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "interactive" => Some(Self::Interactive),
            "normal" => Some(Self::Normal),
            "background" => Some(Self::Background),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    /// O processo terminou (ou o executor foi desligado) com o job incompleto; pode ser repetido.
    Interrupted,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "interrupted" => Self::Interrupted,
            _ => return None,
        })
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl JobError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn cancelled() -> Self {
        Self::new(CODE_CANCELLED, "the job was cancelled")
    }

    pub fn is_cancelled(&self) -> bool {
        self.code == CODE_CANCELLED
    }
}

impl core::fmt::Display for JobError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl core::error::Error for JobError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub done: u64,
    /// 0 = total desconhecido.
    pub total: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub id: JobId,
    pub kind: JobKind,
    pub priority: Priority,
    pub state: JobState,
    pub progress: Progress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_key: Option<String>,
    #[serde(default)]
    pub label: String,
    /// Parâmetros para diagnóstico/retry (o que for restaurável; nunca estado interno).
    #[serde(default)]
    pub params: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
    pub created_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct JobSpec {
    pub kind: JobKind,
    pub priority: Priority,
    pub dedup_key: Option<String>,
    pub label: String,
    pub params: Value,
}

impl JobSpec {
    pub fn new(kind: JobKind, priority: Priority) -> Self {
        Self {
            kind,
            priority,
            dedup_key: None,
            label: String::new(),
            params: Value::Null,
        }
    }

    #[must_use]
    pub fn dedup(mut self, key: impl Into<String>) -> Self {
        self.dedup_key = Some(key.into());
        self
    }

    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    #[must_use]
    pub fn params(mut self, params: Value) -> Self {
        self.params = params;
        self
    }
}

/// Persistência/observação: chamada a cada mudança de estado (e, com limite de taxa, de progresso).
/// Pode ser chamada de qualquer thread; **nunca** com o lock do executor.
pub trait JobSink: Send + Sync {
    fn record(&self, snapshot: &JobSnapshot);
}

#[derive(Clone, Debug)]
pub enum SubmitError {
    QueueFull { priority: Priority, capacity: usize },
    ShuttingDown,
}

impl core::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::QueueFull { priority, capacity } => write!(
                f,
                "JOB_QUEUE_FULL: the {} queue is full ({capacity} jobs)",
                priority.as_str()
            ),
            Self::ShuttingDown => {
                f.write_str("JOB_EXECUTOR_SHUTDOWN: the executor is shutting down")
            }
        }
    }
}

impl core::error::Error for SubmitError {}

/// Token de cancelamento compartilhado entre o chamador e o job (e, em jobs de mídia, repassado
/// ao processo filho).
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Default)]
pub(crate) struct ProgressCell {
    pub done: AtomicU64,
    pub total: AtomicU64,
}

/// Contexto entregue ao job.
pub struct JobCtx {
    pub(crate) id: JobId,
    pub(crate) token: CancelToken,
    pub(crate) progress: Arc<ProgressCell>,
    pub(crate) on_progress: Box<dyn Fn() + Send + Sync>,
}

impl JobCtx {
    pub fn id(&self) -> &JobId {
        &self.id
    }

    pub fn token(&self) -> CancelToken {
        self.token.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// `Err(JOB_CANCELLED)` se o job foi cancelado — chame em pontos seguros do laço.
    pub fn check(&self) -> Result<(), JobError> {
        if self.is_cancelled() {
            Err(JobError::cancelled())
        } else {
            Ok(())
        }
    }

    /// Progresso monotônico (`done` nunca regride) — seguro entre threads.
    pub fn set_progress(&self, done: u64, total: u64) {
        self.progress.total.store(total, Ordering::SeqCst);
        self.progress.done.fetch_max(done, Ordering::SeqCst);
        (self.on_progress)();
    }
}

impl core::fmt::Debug for JobCtx {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("JobCtx")
            .field("id", &self.id)
            .field("cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}
