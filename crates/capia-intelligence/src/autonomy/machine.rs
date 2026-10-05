//! Máquina de estados formal da AI Run (PHASE5_ORCHESTRATOR §2/§3). Enums **fortes** e uma tabela
//! de transição **fechada**: não existe `set_stage("qualquer string")`; toda mudança passa por
//! [`transition`], que rejeita o que a tabela não prevê.

use serde::{Deserialize, Serialize};

/// Estágio de trabalho de uma Run (os estados "de espera" são [`RunStatus`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStage {
    Understand,
    Plan,
    ValidatePlan,
    Acquire,
    Edit,
    Review,
    Correct,
    Done,
}

impl RunStage {
    pub const ALL: [Self; 8] = [
        Self::Understand,
        Self::Plan,
        Self::ValidatePlan,
        Self::Acquire,
        Self::Edit,
        Self::Review,
        Self::Correct,
        Self::Done,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Understand => "understand",
            Self::Plan => "plan",
            Self::ValidatePlan => "validate_plan",
            Self::Acquire => "acquire",
            Self::Edit => "edit",
            Self::Review => "review",
            Self::Correct => "correct",
            Self::Done => "done",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|x| x.as_str() == s)
    }

    /// Estágios em que o Agent pode escrever no documento (só via `preview → apply_plan`).
    pub fn may_write(self) -> bool {
        matches!(self, Self::Edit | Self::Correct)
    }
}

/// Estado da Run como um todo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    WaitingUser,
    Paused,
    Failed,
    Cancelled,
    Completed,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::WaitingUser => "waiting_user",
            Self::Paused => "paused",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Completed => "completed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::Pending,
            Self::Running,
            Self::WaitingUser,
            Self::Paused,
            Self::Failed,
            Self::Cancelled,
            Self::Completed,
        ]
        .into_iter()
        .find(|x| x.as_str() == s)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Failed | Self::Cancelled | Self::Completed)
    }
}

/// Resultado de **uma execução** de stage (o que o handler diz que aconteceu).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Stage concluído sem ressalvas.
    Success,
    /// Precisa de uma decisão humana (pergunta, aprovação, orçamento, conflito).
    NeedsUser,
    /// PLAN: falta contexto crítico — volta a entender.
    MissingContext,
    /// VALIDATE_PLAN: plano inválido, mas corrigível por replanejamento.
    InvalidFixable,
    /// VALIDATE_PLAN: válido e todos os assets existem.
    ValidAssetsOk,
    /// VALIDATE_PLAN: válido, faltam assets a adquirir.
    ValidMissingAssets,
    /// ACQUIRE: tudo pronto.
    AllReady,
    /// ACQUIRE: parcial com fallback possível — replaneja.
    PartialFallback,
    /// ACQUIRE: asset crítico indisponível (sem fallback).
    CriticalUnavailable,
    /// EDIT: aplicado.
    ApplyOk,
    /// EDIT: a revisão do documento mudou (edição manual) — revalidar.
    Drift,
    /// EDIT/CORRECT: conflito estrutural — replanejar.
    Conflict,
    /// REVIEW: aprovado.
    ReviewPass,
    /// REVIEW: achados acionáveis e ainda há ciclos.
    ReviewActionable,
    /// REVIEW: o problema exige replanejamento.
    ReviewReplan,
    /// REVIEW: ciclos/orçamento esgotados.
    ReviewExhausted,
    /// CORRECT: correção aplicada.
    CorrectOk,
    /// CORRECT: correção exige replanejamento.
    CorrectReplan,
    /// Falha não recuperável do stage.
    Failure,
    /// Cancelamento.
    Cancel,
}

/// Para onde a Run vai.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "to", rename_all = "snake_case")]
pub enum Next {
    Go {
        stage: RunStage,
    },
    /// Aguarda decisão; ao decidir, retoma em `resume`.
    Wait {
        resume: RunStage,
    },
    Complete,
    Fail,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IllegalTransition {
    pub from: RunStage,
    pub outcome: Outcome,
}

impl core::fmt::Display for IllegalTransition {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "illegal transition: {} --{:?}-->",
            self.from.as_str(),
            self.outcome
        )
    }
}

impl std::error::Error for IllegalTransition {}

/// Tabela de transição (PHASE5_ORCHESTRATOR §3). **Única** fonte de verdade.
pub fn transition(from: RunStage, outcome: Outcome) -> Result<Next, IllegalTransition> {
    use Outcome as O;
    use RunStage as S;
    let go = |stage| Ok(Next::Go { stage });
    let wait = |resume| Ok(Next::Wait { resume });
    // cancelamento e falha valem em qualquer stage ativo
    if from == S::Done {
        return Err(IllegalTransition { from, outcome });
    }
    match (from, outcome) {
        (_, O::Cancel) => Ok(Next::Cancelled),
        (_, O::Failure) => Ok(Next::Fail),
        (S::Understand, O::Success) => go(S::Plan),
        (S::Understand, O::NeedsUser) => wait(S::Understand),

        (S::Plan, O::Success) => go(S::ValidatePlan),
        (S::Plan, O::MissingContext) => go(S::Understand),
        (S::Plan, O::NeedsUser) => wait(S::Plan),

        (S::ValidatePlan, O::ValidAssetsOk) => go(S::Edit),
        (S::ValidatePlan, O::ValidMissingAssets) => go(S::Acquire),
        (S::ValidatePlan, O::InvalidFixable) => go(S::Plan),
        // espera na validação: ao decidir, revalida (a decisão pode mudar plano/orçamento)
        (S::ValidatePlan, O::NeedsUser) => wait(S::ValidatePlan),

        (S::Acquire, O::AllReady) => go(S::ValidatePlan),
        (S::Acquire, O::PartialFallback) => go(S::Plan),
        (S::Acquire, O::NeedsUser | O::CriticalUnavailable) => wait(S::Acquire),

        (S::Edit, O::ApplyOk) => go(S::Review),
        (S::Edit, O::Drift) => go(S::ValidatePlan),
        (S::Edit, O::Conflict) => go(S::Plan),
        (S::Edit, O::NeedsUser) => wait(S::Edit),

        (S::Review, O::ReviewPass) => Ok(Next::Complete),
        (S::Review, O::ReviewActionable) => go(S::Correct),
        (S::Review, O::ReviewReplan) => go(S::Plan),
        (S::Review, O::ReviewExhausted | O::NeedsUser) => wait(S::Review),

        (S::Correct, O::CorrectOk) => go(S::Review),
        (S::Correct, O::CorrectReplan | O::Conflict) => go(S::Plan),
        (S::Correct, O::NeedsUser) => wait(S::Correct),

        _ => Err(IllegalTransition { from, outcome }),
    }
}

/// Transições aceitas pela tabela (para testes de propriedade/documentação).
pub fn legal_transitions() -> Vec<(RunStage, Outcome, Next)> {
    let outcomes = [
        Outcome::Success,
        Outcome::NeedsUser,
        Outcome::MissingContext,
        Outcome::InvalidFixable,
        Outcome::ValidAssetsOk,
        Outcome::ValidMissingAssets,
        Outcome::AllReady,
        Outcome::PartialFallback,
        Outcome::CriticalUnavailable,
        Outcome::ApplyOk,
        Outcome::Drift,
        Outcome::Conflict,
        Outcome::ReviewPass,
        Outcome::ReviewActionable,
        Outcome::ReviewReplan,
        Outcome::ReviewExhausted,
        Outcome::CorrectOk,
        Outcome::CorrectReplan,
        Outcome::Failure,
        Outcome::Cancel,
    ];
    let mut v = Vec::new();
    for s in RunStage::ALL {
        for o in outcomes {
            if let Ok(n) = transition(s, o) {
                v.push((s, o, n));
            }
        }
    }
    v
}

/// Taxonomia de erros de Run (PHASE5_ORCHESTRATOR §23).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunErrorKind {
    Provider,
    Tool,
    PlanInvalid,
    AssetUnavailable,
    BudgetExceeded,
    Conflict,
    Permission,
    Security,
    Persistence,
    Generation,
    Gateway,
    Cancelled,
    Internal,
}

impl RunErrorKind {
    pub fn recoverable(self) -> bool {
        matches!(
            self,
            Self::Provider
                | Self::PlanInvalid
                | Self::AssetUnavailable
                | Self::BudgetExceeded
                | Self::Conflict
                | Self::Generation
                | Self::Gateway
        )
    }

    /// Vale tentar de novo automaticamente (transitório)?
    pub fn retryable(self) -> bool {
        matches!(self, Self::Provider | Self::Generation | Self::Gateway)
    }

    /// Transição sugerida quando o erro não é retentável.
    pub fn suggested(self) -> Outcome {
        match self {
            Self::PlanInvalid => Outcome::InvalidFixable,
            Self::BudgetExceeded | Self::AssetUnavailable => Outcome::NeedsUser,
            Self::Conflict => Outcome::Conflict,
            Self::Cancelled => Outcome::Cancel,
            _ => Outcome::Failure,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn the_happy_path_is_legal_and_ordered() {
        let path = [
            (RunStage::Understand, Outcome::Success, RunStage::Plan),
            (RunStage::Plan, Outcome::Success, RunStage::ValidatePlan),
            (
                RunStage::ValidatePlan,
                Outcome::ValidAssetsOk,
                RunStage::Edit,
            ),
            (RunStage::Edit, Outcome::ApplyOk, RunStage::Review),
        ];
        for (from, o, to) in path {
            assert_eq!(transition(from, o).unwrap(), Next::Go { stage: to });
        }
        assert_eq!(
            transition(RunStage::Review, Outcome::ReviewPass).unwrap(),
            Next::Complete
        );
    }

    #[test]
    fn nothing_leaves_done_and_unknown_pairs_are_rejected() {
        for o in [Outcome::Success, Outcome::Cancel, Outcome::Failure] {
            assert!(transition(RunStage::Done, o).is_err());
        }
        assert!(transition(RunStage::Understand, Outcome::ApplyOk).is_err());
        assert!(transition(RunStage::Plan, Outcome::ReviewPass).is_err());
        assert!(transition(RunStage::Edit, Outcome::Success).is_err());
    }

    #[test]
    fn every_legal_transition_lands_on_a_known_place() {
        for (from, _o, next) in legal_transitions() {
            if let Next::Go { stage } = next {
                assert_ne!(stage, RunStage::Done, "Done is only reached via Complete");
            }
            if let Next::Wait { resume } = next {
                assert_eq!(resume, from, "a wait resumes where it stopped");
            }
        }
    }

    #[test]
    fn only_edit_and_correct_may_write() {
        let w: Vec<_> = RunStage::ALL
            .into_iter()
            .filter(|s| s.may_write())
            .collect();
        assert_eq!(w, vec![RunStage::Edit, RunStage::Correct]);
    }

    #[test]
    fn stage_and_status_names_round_trip() {
        for s in RunStage::ALL {
            assert_eq!(RunStage::parse(s.as_str()), Some(s));
        }
        assert!(RunStage::parse("anything").is_none());
        assert_eq!(
            RunStatus::parse("waiting_user"),
            Some(RunStatus::WaitingUser)
        );
    }
}
