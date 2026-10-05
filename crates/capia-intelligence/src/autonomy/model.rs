//! Modelo persistido de uma AI Run (PHASE5_ORCHESTRATOR §4..§16): entradas, política, orçamento,
//! uso, decisões pendentes, aprovações e o cursor. Tudo serializável e versionado; nada aqui é
//! documento da timeline nem entra no undo.

use super::machine::{RunStage, RunStatus};
use super::plan::{EditPlan, ProductionPlan, ValidationReport};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const RUN_SCHEMA_VERSION: u32 = 1;

// ---- entradas ---------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliverableRequest {
    pub key: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub max_duration_s: Option<u32>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantRequest {
    /// Quantidade de variantes desejadas (ganchos/CTAs/ritmos/formatos), além do master.
    #[serde(default)]
    pub count: u32,
    /// `hooks` | `ctas` | `pacing` | `formats`.
    #[serde(default)]
    pub axis: Vec<String>,
}

/// O que o usuário entrega à Run: brief + bruto + referência + objetivos/formatos.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInputs {
    #[serde(default)]
    pub brief_text: Option<String>,
    /// Caminhos de documentos (DOCX/PDF/TXT/MD) a interpretar.
    #[serde(default)]
    pub documents: Vec<String>,
    /// Assets de mídia bruta já importados (ids).
    #[serde(default)]
    pub assets: Vec<String>,
    /// Assets de vídeos de referência (ids).
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub deliverables: Vec<DeliverableRequest>,
    #[serde(default)]
    pub variants: Option<VariantRequest>,
    /// Cliente explícito (memória de cliente nunca é inferida por nome).
    #[serde(default)]
    pub client_id: Option<String>,
    /// Reaproveita um DemandSpec já criado/aprovado.
    #[serde(default)]
    pub demand_spec_id: Option<String>,
    /// Respostas do usuário às perguntas abertas (índice → texto).
    #[serde(default)]
    pub answers: Vec<String>,
    /// Sequence já existente (master de uma Run anterior) que as variantes reaproveitam.
    #[serde(default)]
    pub master_sequence: Option<String>,
}

// ---- política / orçamento / uso ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecApproval {
    Auto,
    RequiredIfQuestions,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanApproval {
    Auto,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquireSource {
    Project,
    Library,
    Gateway,
    Generate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriticalUnavailable {
    Wait,
    Fail,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunPolicy {
    pub demand_spec: SpecApproval,
    pub plan: PlanApproval,
    /// Gasto estimado acima disto exige aprovação **sempre** (micros).
    pub spend_threshold_micros: u64,
    pub generation_requires_approval: bool,
    /// Remoções acima disto (clips/tracks) exigem aprovação do plano.
    pub destructive_threshold: u32,
    pub final_approval: bool,
    /// Licença desconhecida exige aprovação (padrão seguro).
    pub unknown_license_requires_approval: bool,
    /// Licença conhecida como restrita: `true` descarta o candidato; `false` pede decisão.
    pub reject_restricted_license: bool,
    pub allow_gateway: bool,
    pub allow_generation: bool,
    pub on_critical_unavailable: CriticalUnavailable,
    /// Memória de projeto proposta pela IA pode ser ativada pela política do projeto.
    pub project_memory_auto_activate: bool,
    pub acquire_order: Vec<AcquireSource>,
    /// Rodadas de perguntas ao usuário antes de seguir mesmo com dúvidas abertas.
    pub max_question_rounds: u32,
}

impl Default for RunPolicy {
    fn default() -> Self {
        Self {
            demand_spec: SpecApproval::RequiredIfQuestions,
            plan: PlanApproval::Always,
            spend_threshold_micros: 1_000_000,
            generation_requires_approval: true,
            destructive_threshold: 25,
            final_approval: false,
            unknown_license_requires_approval: true,
            reject_restricted_license: true,
            allow_gateway: true,
            allow_generation: true,
            on_critical_unavailable: CriticalUnavailable::Wait,
            project_memory_auto_activate: false,
            acquire_order: vec![
                AcquireSource::Project,
                AcquireSource::Library,
                AcquireSource::Gateway,
                AcquireSource::Generate,
            ],
            max_question_rounds: 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunBudget {
    pub max_cost_micros: Option<u64>,
    pub max_tokens: Option<u64>,
    pub max_provider_calls: Option<u32>,
    pub max_generations: Option<u32>,
    pub max_review_loops: u32,
    pub max_replans: u32,
    pub max_wall_time_ms: Option<u64>,
}

impl Default for RunBudget {
    fn default() -> Self {
        Self {
            max_cost_micros: None,
            max_tokens: None,
            max_provider_calls: None,
            max_generations: None,
            max_review_loops: 2,
            max_replans: 3,
            max_wall_time_ms: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunUsage {
    pub cost_micros: u64,
    /// Chamadas cujo preço não foi declarado (custo desconhecido ≠ zero).
    pub unknown_cost_calls: u32,
    pub tokens: u64,
    pub provider_calls: u32,
    pub generations: u32,
    pub review_loops: u32,
    pub replans: u32,
    pub wall_time_ms: u64,
}

/// Qual limite estourou (para a mensagem de `BudgetExtension`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetLimit {
    Cost,
    Tokens,
    ProviderCalls,
    Generations,
    ReviewLoops,
    Replans,
    WallTime,
}

impl RunBudget {
    /// Primeiro limite já atingido (≥) ou `None`. `review_loops`/`replans` são tratados por quem
    /// decide a transição (limite = esgotado quando `usage >= max`).
    pub fn exceeded(&self, u: &RunUsage) -> Option<BudgetLimit> {
        if self.max_cost_micros.is_some_and(|m| u.cost_micros > m) {
            return Some(BudgetLimit::Cost);
        }
        if self.max_tokens.is_some_and(|m| u.tokens > m) {
            return Some(BudgetLimit::Tokens);
        }
        if self
            .max_provider_calls
            .is_some_and(|m| u.provider_calls > m)
        {
            return Some(BudgetLimit::ProviderCalls);
        }
        if self.max_generations.is_some_and(|m| u.generations > m) {
            return Some(BudgetLimit::Generations);
        }
        if self.max_wall_time_ms.is_some_and(|m| u.wall_time_ms > m) {
            return Some(BudgetLimit::WallTime);
        }
        None
    }

    pub fn replans_left(&self, u: &RunUsage) -> bool {
        u.replans < self.max_replans
    }

    pub fn review_loops_left(&self, u: &RunUsage) -> bool {
        u.review_loops < self.max_review_loops
    }
}

// ---- decisões -----------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    OpenQuestion,
    PlanApproval,
    SpendApproval,
    AssetApproval,
    GenerationApproval,
    ConflictResolution,
    BudgetExtension,
    FinalApproval,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionOption {
    pub id: String,
    pub label: String,
}

impl DecisionOption {
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingDecision {
    pub id: String,
    pub kind: DecisionKind,
    pub question: String,
    pub options: Vec<DecisionOption>,
    #[serde(default)]
    pub context: Value,
    pub consequences: String,
    #[serde(default)]
    pub default_option: Option<String>,
    #[serde(default)]
    pub expires_ms: Option<u64>,
    /// Digest do plano/estado ao qual a decisão está **presa**: mudou ⇒ decisão velha.
    #[serde(default)]
    pub bound_digest: Option<String>,
    pub resume_stage: RunStage,
    pub created_ms: u64,
}

/// Registro auditável de uma decisão tomada.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub decision_id: String,
    pub kind: DecisionKind,
    pub option: String,
    pub actor: String,
    pub at_ms: u64,
    #[serde(default)]
    pub bound_digest: Option<String>,
    #[serde(default)]
    pub detail: Value,
}

// ---- cursor / erro ------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunErrorInfo {
    pub kind: super::machine::RunErrorKind,
    pub code: String,
    /// Mensagem já redigida (nunca carrega segredo).
    pub message: String,
    pub stage: RunStage,
    pub recoverable: bool,
}

/// Transação aplicada pela Run (para auditoria e undo seletivo).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedRef {
    pub stage: RunStage,
    pub deliverable: String,
    pub entry_id: u64,
    pub revision_before: u64,
    pub revision_after: u64,
    pub operation_namespace: String,
    pub cycle: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRef {
    pub id: String,
    pub scope: String,
    /// Digest do conteúdo no momento do uso (auditoria sem reconstruir conteúdo apagado).
    pub content_digest: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AiRun {
    pub schema_version: u32,
    pub id: String,
    pub project: String,
    pub status: RunStatus,
    pub stage: RunStage,
    pub revision: u64,
    pub brain_profile_id: String,
    pub demand_spec_id: Option<String>,
    pub demand_spec_version: Option<u32>,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub started_ms: Option<u64>,
    pub completed_ms: Option<u64>,
    pub inputs: RunInputs,
    pub policy: RunPolicy,
    pub budget: RunBudget,
    pub usage: RunUsage,
    /// Nº de entradas em stages (idempotency key dos stages).
    pub current_attempt: u32,
    pub question_rounds: u32,
    pub parent_run_id: Option<String>,
    pub variant_group_id: Option<String>,
    pub production_plan: Option<ProductionPlan>,
    pub edit_plans: Vec<EditPlan>,
    pub validation: Option<ValidationReport>,
    pub applied: Vec<AppliedRef>,
    pub reviews: Vec<ReviewRef>,
    pub pending: Option<PendingDecision>,
    pub approvals: Vec<ApprovalRecord>,
    pub memory_used: Vec<MemoryRef>,
    pub error: Option<RunErrorInfo>,
    /// Quando em espera/pausa: stage em que retomar.
    pub resume_stage: Option<RunStage>,
    /// Progresso parcial do stage (batch já aplicado, etc.) — retomada sabe onde parou.
    pub checkpoint: Value,
    /// Histórico dos digests de plano (para barrar replanejamento sem progresso).
    pub plan_digests: Vec<String>,
    /// Resumo final (somente em `Completed`).
    pub report: Option<Value>,
    /// Sequences produzidas (por deliverable).
    pub sequences: Vec<ProducedSequence>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducedSequence {
    pub deliverable: String,
    pub sequence_id: String,
    pub role: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewRef {
    pub id: String,
    pub revision: u64,
    pub score: f64,
    pub pass: bool,
    pub findings: u32,
    pub cycle: u32,
}

impl AiRun {
    pub fn new(
        id: String,
        project: String,
        inputs: RunInputs,
        brain_profile_id: String,
        now: u64,
    ) -> Self {
        Self {
            schema_version: RUN_SCHEMA_VERSION,
            id,
            project,
            status: RunStatus::Pending,
            stage: RunStage::Understand,
            revision: 0,
            brain_profile_id,
            demand_spec_id: inputs.demand_spec_id.clone(),
            demand_spec_version: None,
            created_ms: now,
            updated_ms: now,
            started_ms: None,
            completed_ms: None,
            inputs,
            policy: RunPolicy::default(),
            budget: RunBudget::default(),
            usage: RunUsage::default(),
            current_attempt: 0,
            question_rounds: 0,
            parent_run_id: None,
            variant_group_id: None,
            production_plan: None,
            edit_plans: Vec::new(),
            validation: None,
            applied: Vec::new(),
            reviews: Vec::new(),
            pending: None,
            approvals: Vec::new(),
            memory_used: Vec::new(),
            error: None,
            resume_stage: None,
            checkpoint: Value::Null,
            plan_digests: Vec::new(),
            report: None,
            sequences: Vec::new(),
        }
    }

    /// Ator das escritas desta Run (só `preview → apply_plan`; os tokens ficam presos a ele).
    pub fn actor_id(&self) -> String {
        format!("run:{}", self.id)
    }
}
