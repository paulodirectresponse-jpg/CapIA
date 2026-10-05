//! Propriedades da autonomia (gerador próprio, semente fixa; `CAPIA_AUTONOMY_PROP_CASES` aumenta):
//! transições legais, orçamento nunca acima do teto, promoção de memória só com aprovação humana.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_intelligence::autonomy::machine::{
    Next, Outcome, RunStage, legal_transitions, transition,
};
use capia_intelligence::autonomy::memory::{
    EvidenceRef, MemoryDraft, MemoryKind, MemoryManager, MemoryScope, MemorySource, MemoryStatus,
    UserApproval,
};
use capia_store::{AppDb, AutonomyStore};
use common::*;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn cases() -> u64 {
    std::env::var("CAPIA_AUTONOMY_PROP_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200)
}

#[test]
fn random_walks_over_the_state_machine_never_break_the_write_gates() {
    let table = legal_transitions();
    let mut rng = Rng(0x5eed_f00d_1234_abcd);
    for case in 0..cases() {
        let mut stage = RunStage::Understand;
        let mut prev: Option<RunStage> = None;
        let mut terminal = false;
        for _ in 0..300 {
            let legal: Vec<_> = table.iter().filter(|(s, _, _)| *s == stage).collect();
            assert!(!legal.is_empty(), "case {case}: no way out of {stage:?}");
            let (_, outcome, next) = legal[rng.below(legal.len() as u64) as usize];
            assert_eq!(transition(stage, *outcome).unwrap(), *next);
            match next {
                Next::Go { stage: to } => {
                    // nada escreve sem passar por VALIDATE_PLAN; REVIEW só depois de escrever
                    if *to == RunStage::Edit {
                        assert_eq!(
                            stage,
                            RunStage::ValidatePlan,
                            "case {case}: edit reached from {stage:?}"
                        );
                    }
                    if *to == RunStage::Review {
                        assert!(
                            matches!(stage, RunStage::Edit | RunStage::Correct),
                            "case {case}"
                        );
                    }
                    if *to == RunStage::Correct {
                        assert_eq!(stage, RunStage::Review, "case {case}");
                    }
                    prev = Some(stage);
                    stage = *to;
                }
                Next::Wait { resume } => {
                    // esperar o humano nunca escreve: retoma no mesmo stage que pediu
                    assert_eq!(*resume, stage, "case {case}");
                }
                Next::Complete | Next::Fail | Next::Cancelled => {
                    terminal = true;
                    break;
                }
            }
        }
        let _ = (prev, terminal);
        // de qualquer stage ativo, cancelar e falhar sempre são legais; de DONE nada é
        for s in [
            RunStage::Understand,
            RunStage::Plan,
            RunStage::ValidatePlan,
            RunStage::Acquire,
            RunStage::Edit,
            RunStage::Review,
            RunStage::Correct,
        ] {
            assert_eq!(transition(s, Outcome::Cancel).unwrap(), Next::Cancelled);
            assert_eq!(transition(s, Outcome::Failure).unwrap(), Next::Fail);
        }
        assert!(transition(RunStage::Done, Outcome::Success).is_err());
    }
}

fn project(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let w = world_full(name, None, vec![], false, None).expect("world");
    let dir = w.dir.clone();
    std::mem::forget(w);
    (dir.join("p.capia"), dir)
}

#[test]
fn random_ledger_operations_never_commit_above_the_limit() {
    let (proj, _dir) = project("prop-ledger");
    let s = AutonomyStore::open(&proj, Duration::from_secs(5)).unwrap();
    let mut rng = Rng(0x1ced_6e55_9d3b_77a1);
    for case in 0..cases().min(120) {
        let run = format!("r{case}");
        let limit = 500 + rng.below(2_000);
        let mut open: Vec<(String, u64)> = Vec::new();
        for step in 0..40 {
            match rng.below(3) {
                0 => {
                    let est = 1 + rng.below(700);
                    let key = format!("k{step}");
                    if s.ledger_reserve(&run, &key, est, Some(limit), &json!({}), 1)
                        .unwrap()
                    {
                        open.push((key, est));
                    }
                }
                1 if !open.is_empty() => {
                    let (k, est) = open.remove(rng.below(open.len() as u64) as usize);
                    // gasto real nunca acima do reservado (o provider respeita o teto pedido)
                    s.ledger_settle(&run, &k, rng.below(est + 1), &json!({}), 2)
                        .unwrap();
                }
                _ if !open.is_empty() => {
                    let (k, _) = open.remove(rng.below(open.len() as u64) as usize);
                    s.ledger_release(&run, &k, 3).unwrap();
                }
                _ => {}
            }
            assert!(
                s.ledger_committed(&run).unwrap() <= limit,
                "case {case}: committed above the limit {limit}"
            );
        }
    }
}

#[test]
fn random_memory_operations_never_activate_user_or_client_without_a_human() {
    let (proj, dir) = project("prop-memory");
    let store = Arc::new(AutonomyStore::open(&proj, Duration::from_secs(5)).unwrap());
    let app = Arc::new(AppDb::open(&dir.join("app.db"), Duration::from_secs(5)).unwrap());
    let m = MemoryManager::new(store, Some(app));
    let mut rng = Rng(0x0dd5_eed5_4242_0007);
    let scopes = [MemoryScope::User, MemoryScope::Client, MemoryScope::Project];
    let mut approved_ids = std::collections::BTreeSet::new();
    for n in 0..cases().min(150) {
        let scope = scopes[rng.below(3) as usize];
        let client = (scope == MemoryScope::Client).then(|| format!("c{}", rng.below(3)));
        let d = MemoryDraft {
            scope,
            client_id: client,
            kind: MemoryKind::Preference,
            content: format!("rule number {n} {}", rng.below(5)),
            structured: None,
            source: MemorySource::Agent,
            confidence: 1.0,
            evidence: vec![EvidenceRef {
                kind: "run".into(),
                detail: format!("e{n}"),
            }],
            key: None,
        };
        let it = m.propose(d, Some("r"), rng.below(2) == 0).unwrap();
        if rng.below(4) == 0 {
            // só aqui entra um humano
            let target = scopes[rng.below(3) as usize];
            let cid = (target == MemoryScope::Client).then_some("c0");
            if let Ok(a) = m.approve(
                &it.id,
                Some(target),
                cid,
                None,
                &UserApproval::explicit_from_ui("user"),
            ) {
                approved_ids.insert(a.id);
            }
        }
        // invariante global
        for i in m.list(None, Some(MemoryStatus::Active)).unwrap() {
            if i.scope.needs_human() {
                assert!(
                    approved_ids.contains(&i.id),
                    "case {n}: {:?} active without a human",
                    i.scope
                );
            }
        }
    }
}
