//! Uso, custo e orçamento. Custo desconhecido **permanece desconhecido** (nunca estimado de cabeça);
//! retry conta cada tentativa que gastou tokens; cache hit não gera gasto novo de provider.

use crate::registry::Pricing;
use crate::types::Usage;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Cost {
    pub known: bool,
    /// Micro-unidades da moeda (arredondado para **cima**: nunca subestima).
    pub micros: u64,
    pub currency: Option<String>,
}

impl Cost {
    pub fn unknown() -> Self {
        Self::default()
    }

    pub fn zero(currency: Option<String>) -> Self {
        Self {
            known: true,
            micros: 0,
            currency,
        }
    }

    pub fn add(&mut self, other: &Cost) {
        match (self.known, other.known) {
            (true, true) => {
                self.micros = self.micros.saturating_add(other.micros);
                if self.currency.is_none() {
                    self.currency.clone_from(&other.currency);
                }
            }
            // somar conhecido + desconhecido ⇒ o total fica "parcial": marcamos desconhecido
            (_, false) => self.known = false,
            (false, true) => {}
        }
    }
}

fn ceil_div(n: u128, d: u128) -> u128 {
    n.div_ceil(d)
}

/// Custo de um `Usage` segundo o preço declarado; sem preço ⇒ desconhecido.
pub fn cost_of(pricing: Option<&Pricing>, usage: &Usage) -> Cost {
    let Some(p) = pricing else {
        return Cost::unknown();
    };
    let cached = usage.cached_input_tokens.min(usage.input_tokens);
    let fresh = usage.input_tokens - cached;
    let cached_price = p
        .cached_input_micros_per_mtok
        .unwrap_or(p.input_micros_per_mtok);
    let total = u128::from(fresh) * u128::from(p.input_micros_per_mtok)
        + u128::from(cached) * u128::from(cached_price)
        + u128::from(usage.output_tokens) * u128::from(p.output_micros_per_mtok);
    Cost {
        known: true,
        micros: u64::try_from(ceil_div(total, 1_000_000)).unwrap_or(u64::MAX),
        currency: Some(p.currency.clone()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallStatus {
    Ok,
    Failed,
    Cancelled,
    CacheHit,
}

/// Registro de uma tentativa de chamada (sem segredo, sem conteúdo de prompt).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    pub request_id: String,
    pub task_id: Option<String>,
    pub provider_id: String,
    pub endpoint_id: String,
    pub model_id: String,
    pub capability: String,
    pub purpose: Option<String>,
    pub usage: Usage,
    pub cost: Cost,
    pub latency_ms: u64,
    /// Número da tentativa dentro da chamada lógica (1 = primeira).
    pub attempt: u32,
    pub status: CallStatus,
    pub error_code: Option<String>,
    pub pricing_date: Option<String>,
    pub timestamp_ms: u64,
}

pub trait UsageSink: Send + Sync + core::fmt::Debug {
    fn record(&self, r: &UsageRecord);
}

#[derive(Default)]
pub struct MemorySink {
    pub records: Mutex<Vec<UsageRecord>>,
}

impl core::fmt::Debug for MemorySink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MemorySink").finish_non_exhaustive()
    }
}

impl UsageSink for MemorySink {
    fn record(&self, r: &UsageRecord) {
        if let Ok(mut v) = self.records.lock() {
            v.push(r.clone());
        }
    }
}

impl MemorySink {
    pub fn snapshot(&self) -> Vec<UsageRecord> {
        self.records.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

/// Orçamento do BrainProfile (por tarefa; o acumulado mensal é do app DB).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Budgets {
    #[serde(default)]
    pub currency: Option<String>,
    /// Teto de custo por tarefa (só aplicável com preço conhecido).
    #[serde(default)]
    pub max_cost_per_task_micros: Option<u64>,
    /// Acima disso, a UI avisa.
    #[serde(default)]
    pub warn_at_micros: Option<u64>,
    /// Até aqui, aplica sem pedir aprovação.
    #[serde(default)]
    pub auto_approve_below_micros: Option<u64>,
    /// Teto de tokens por tarefa (vale também quando o preço é desconhecido).
    #[serde(default)]
    pub max_tokens_per_task: Option<u64>,
    #[serde(default)]
    pub max_calls_per_task: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Spent {
    pub cost: Cost,
    pub tokens: u64,
    pub calls: u32,
    pub unknown_cost_calls: u32,
}

/// Acumulador por tarefa; `check` antes de chamar, `charge` depois.
#[derive(Debug)]
pub struct BudgetTracker {
    budgets: Budgets,
    spent: Mutex<Spent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BudgetCheck {
    Ok,
    /// Passou do limiar de aviso (continua permitido).
    Warn(String),
    Exceeded(String),
}

impl BudgetTracker {
    pub fn new(budgets: Budgets) -> Self {
        Self {
            budgets,
            spent: Mutex::new(Spent::default()),
        }
    }

    pub fn spent(&self) -> Spent {
        self.spent.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// `est_cost`: custo estimado **conhecido** da próxima chamada, se houver.
    pub fn check(&self, est_cost: Option<u64>) -> BudgetCheck {
        let s = self.spent();
        if let Some(max) = self.budgets.max_calls_per_task
            && s.calls >= max
        {
            return BudgetCheck::Exceeded(format!("max {max} calls per task reached"));
        }
        if let Some(max) = self.budgets.max_tokens_per_task
            && s.tokens >= max
        {
            return BudgetCheck::Exceeded(format!("max {max} tokens per task reached"));
        }
        if let Some(max) = self.budgets.max_cost_per_task_micros {
            let projected = s.cost.micros.saturating_add(est_cost.unwrap_or(0));
            if s.cost.known && projected > max {
                return BudgetCheck::Exceeded(format!(
                    "task cost budget of {max} micro-units would be exceeded"
                ));
            }
        }
        if let Some(w) = self.budgets.warn_at_micros
            && s.cost.known
            && s.cost.micros.saturating_add(est_cost.unwrap_or(0)) >= w
        {
            return BudgetCheck::Warn("task spending passed the warning threshold".into());
        }
        BudgetCheck::Ok
    }

    pub fn charge(&self, usage: &Usage, cost: &Cost) {
        if let Ok(mut s) = self.spent.lock() {
            s.calls += 1;
            s.tokens = s
                .tokens
                .saturating_add(usage.input_tokens + usage.output_tokens);
            if cost.known {
                if s.calls == 1 || s.cost.known {
                    s.cost.known = true;
                    s.cost.micros = s.cost.micros.saturating_add(cost.micros);
                    if s.cost.currency.is_none() {
                        s.cost.currency.clone_from(&cost.currency);
                    }
                }
            } else {
                s.unknown_cost_calls += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn pricing() -> Pricing {
        Pricing {
            currency: "USD".into(),
            input_micros_per_mtok: 3_000_000,
            output_micros_per_mtok: 15_000_000,
            cached_input_micros_per_mtok: Some(300_000),
            audio_micros_per_second: None,
            image_micros_each: None,
            source: "teste".into(),
            effective_date: "2026-01-01".into(),
        }
    }

    #[test]
    fn cost_is_exact_ceiling_and_unknown_stays_unknown() {
        let u = Usage {
            input_tokens: 1_000,
            output_tokens: 500,
            cached_input_tokens: 400,
            synthetic: false,
        };
        // 600 frescos·3 + 400 cacheados·0,3 + 500 saída·15 (micro-USD por mil tokens) = 1800+120+7500
        let c = cost_of(Some(&pricing()), &u);
        assert!(c.known);
        assert_eq!(c.micros, 9_420);
        let tiny = Usage {
            input_tokens: 1,
            ..Usage::default()
        };
        assert_eq!(
            cost_of(Some(&pricing()), &tiny).micros,
            3,
            "ceil(3_000_000/1e6)"
        );
        let sub = Usage {
            output_tokens: 1,
            ..Usage::default()
        };
        assert_eq!(cost_of(Some(&pricing()), &sub).micros, 15);
        assert_eq!(cost_of(None, &u), Cost::unknown());
        assert_eq!(c.currency.as_deref(), Some("USD"));
    }

    #[test]
    fn retry_charges_each_attempt_and_budget_caps() {
        let t = BudgetTracker::new(Budgets {
            max_calls_per_task: Some(2),
            ..Budgets::default()
        });
        let u = Usage {
            input_tokens: 10,
            output_tokens: 5,
            ..Usage::default()
        };
        assert_eq!(t.check(None), BudgetCheck::Ok);
        t.charge(&u, &Cost::unknown());
        t.charge(&u, &Cost::unknown());
        assert!(matches!(t.check(None), BudgetCheck::Exceeded(_)));
        let s = t.spent();
        assert_eq!(s.calls, 2);
        assert_eq!(s.unknown_cost_calls, 2);
        assert!(!s.cost.known, "gasto desconhecido não vira zero");
    }

    #[test]
    fn cost_budget_blocks_only_with_known_cost() {
        let t = BudgetTracker::new(Budgets {
            max_cost_per_task_micros: Some(100),
            ..Budgets::default()
        });
        t.charge(
            &Usage::default(),
            &Cost {
                known: true,
                micros: 90,
                currency: Some("USD".into()),
            },
        );
        assert_eq!(t.check(Some(5)), BudgetCheck::Ok);
        assert!(matches!(t.check(Some(20)), BudgetCheck::Exceeded(_)));
    }
}
