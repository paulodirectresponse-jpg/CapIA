//! Handlers de stage da AI Run: UNDERSTAND · PLAN · VALIDATE_PLAN · ACQUIRE · EDIT · REVIEW ·
//! CORRECT. Cada handler devolve um [`Outcome`]; **quem decide a transição é a tabela**
//! (`machine::transition`). Efeitos colaterais (LLM, download, geração, import, apply) passam pelo
//! livro de efeitos (`claim_effect`) com chave determinística: retry/resume nunca duplicam.
//!
//! Escrita no documento: só `Edit` e `Correct`, só por `preview → apply_plan` com o ator da Run, e
//! só **depois** que `ValidatePlan` produziu um relatório válido (e as aprovações exigidas).

use super::critic::{self, BriefFacts, CheckInput, FindingStatus, Review};
use super::failpoint::fp;
use super::gateway::{self, Candidate, GatewayKind, LicenseVerdict, SearchRequest};
use super::generation::{self, GenKind, GenRequest, GenStatus};
use super::machine::{Outcome, RunStage};
use super::memory::{EvidenceRef, MemoryDraft, MemoryKind, MemoryQuery, MemoryScope, MemorySource};
use super::model::{
    AcquireSource, AiRun, AppliedRef, BudgetLimit, CriticalUnavailable, DecisionKind,
    DecisionOption, MemoryRef, PendingDecision, PlanApproval, ProducedSequence, ReviewRef,
    SpecApproval,
};
use super::orchestrator::{Flags, Orchestrator, StageResult, store_err};
use super::plan::{
    AssetKind, AssetNeed, CompileEnv, CostEstimate, EditPlan, Inventory, NeedStatus,
    ProductionPlan, TxUnit, UnitValidation, ValidationReport, compile_production,
    destructive_count, digest_value, validate_edit_plan_semantics,
};
use super::roles::{self, CriticContext, PlanningContext, RoleMeta};
use crate::ctx::IntelCtx;
use crate::demand::{DemandSpec, InterpretOptions};
use crate::docs::{self, DocKind};
use crate::error::{IntelError, IntelResult};
use crate::records::{KIND_DEMAND, KIND_REFERENCE, KIND_TRANSCRIPT, now_ms};
use crate::reference::{self, ReferenceOptions};
use crate::transcript::{self, TranscribeParams};
use capia_ai::dispatcher::TaskCtx;
use capia_store::Claim;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

pub const KIND_RUN_PLAN: &str = "run_production_plan";
pub const KIND_RUN_EDIT_PLAN: &str = "run_edit_plan";
pub const KIND_RUN_VALIDATION: &str = "run_validation";
pub const KIND_RUN_REVIEW: &str = "run_review";

fn short(s: &str) -> String {
    digest_value(&json!(s)).chars().skip(7).take(12).collect()
}

fn texts(items: &[crate::demand::SpecItem]) -> Vec<String> {
    items.iter().map(|i| i.text.clone()).collect()
}

fn field(f: &crate::demand::SpecField) -> Value {
    json!(f.value)
}

pub fn demand_json(s: &DemandSpec) -> Value {
    json!({
        "id": s.id, "version": s.version, "title": s.title,
        "product": field(&s.product), "audience": field(&s.audience), "offer": field(&s.offer),
        "objective": field(&s.objective), "tone": field(&s.tone), "platform": field(&s.platform),
        "duration_and_format": field(&s.duration_and_format), "cta": field(&s.cta),
        "key_claims": texts(&s.key_claims), "must_include": texts(&s.must_include),
        "must_avoid": texts(&s.must_avoid), "constraints": texts(&s.constraints),
        "assets_mentioned": texts(&s.assets_mentioned),
        "open_questions": s.open_questions.iter().map(|q| q.question.clone()).collect::<Vec<_>>(),
    })
}

fn brief_facts(s: &DemandSpec, run: &AiRun) -> BriefFacts {
    let max_s = run
        .inputs
        .deliverables
        .iter()
        .filter_map(|d| d.max_duration_s)
        .min();
    BriefFacts {
        cta: s.cta.value.clone().filter(|c| !c.trim().is_empty()),
        max_duration_ticks: max_s.map(|s| i64::from(s) * 705_600_000),
        must_include: texts(&s.must_include),
        must_avoid: texts(&s.must_avoid),
    }
}

#[allow(clippy::too_many_arguments)]
fn pending(
    run: &AiRun,
    id: String,
    kind: DecisionKind,
    question: &str,
    options: Vec<DecisionOption>,
    context: Value,
    consequences: &str,
    bound: Option<String>,
    resume: RunStage,
    default: Option<&str>,
) -> PendingDecision {
    let _ = run;
    PendingDecision {
        id,
        kind,
        question: question.to_owned(),
        options,
        context,
        consequences: consequences.to_owned(),
        default_option: default.map(str::to_owned),
        expires_ms: None,
        bound_digest: bound,
        resume_stage: resume,
        created_ms: now_ms(),
    }
}

enum NeedResult {
    Resolved(String),
    Wait(Box<PendingDecision>),
    Unavailable(String),
}

impl Orchestrator {
    // ---- despacho -----------------------------------------------------------------------------

    pub(super) async fn run_stage(
        &self,
        run: &mut AiRun,
        ic: &IntelCtx,
        task: &TaskCtx,
        flags: &Flags,
    ) -> IntelResult<StageResult> {
        match run.stage {
            RunStage::Understand => self.st_understand(run, ic, task).await,
            RunStage::Plan => self.st_plan(run, ic, task).await,
            RunStage::ValidatePlan => self.st_validate(run, ic).await,
            RunStage::Acquire => self.st_acquire(run, ic, task, flags).await,
            RunStage::Edit => self.st_edit(run, ic, flags).await,
            RunStage::Review => self.st_review(run, ic, task).await,
            RunStage::Correct => self.st_correct(run, ic, flags).await,
            RunStage::Done => Err(IntelError::new(
                "ILLEGAL_TRANSITION",
                "a done run has no stage to run",
            )),
        }
    }

    pub(super) fn inventory(&self) -> IntelResult<Inventory> {
        Ok(Inventory::from_assets_list(
            &self.deps.engine.read("assets.list", json!({}))?,
        ))
    }

    fn load_spec(&self, ic: &IntelCtx, run: &AiRun) -> IntelResult<Option<DemandSpec>> {
        let Some(id) = &run.demand_spec_id else {
            return Ok(None);
        };
        ic.records()?.latest::<DemandSpec>(KIND_DEMAND, id)
    }

    fn set_checkpoint(run: &mut AiRun, key: &str, v: Value) {
        if !run.checkpoint.is_object() {
            run.checkpoint = json!({});
        }
        run.checkpoint[key] = v;
    }

    /// Lança no livro-razão o custo de uma chamada que não passa pelos papéis (STT etc.).
    fn ledger_other(&self, run_id: &str, key: &str, kind: &str, cost: Option<u64>) {
        let _ = self
            .store
            .ledger_reserve(run_id, key, 0, None, &json!({"kind": kind}), now_ms());
        let _ = self.store.ledger_settle(
            run_id,
            key,
            cost.unwrap_or(0),
            &json!({"kind": kind, "unknown": cost.is_none(), "tokens": 0}),
            now_ms(),
        );
    }

    fn approved(run: &AiRun, kind: DecisionKind, bound: &str) -> bool {
        run.approvals.iter().any(|a| {
            a.kind == kind && a.option == "approve" && a.bound_digest.as_deref() == Some(bound)
        })
    }

    /// Digest ao qual PlanApproval/SpendApproval ficam presos: plano de produção + EditPlans
    /// (estrutura). Mudou ⇒ a aprovação não vale mais.
    pub(super) fn plan_bound(&self, run: &AiRun) -> String {
        digest_value(&json!({
            "production": run.production_plan.as_ref().map(ProductionPlan::digest),
            "edits": run.edit_plans.iter().map(EditPlan::digest).collect::<Vec<_>>(),
            "spec": run.demand_spec_version,
        }))
    }

    pub(super) fn current_bound_digest(&self, run: &AiRun, kind: DecisionKind) -> Option<String> {
        match kind {
            DecisionKind::PlanApproval | DecisionKind::SpendApproval => Some(self.plan_bound(run)),
            _ => run.pending.as_ref().and_then(|p| p.bound_digest.clone()),
        }
    }

    fn actor(run: &AiRun) -> capia_commands::Actor {
        capia_commands::Actor::agent(run.actor_id())
    }

    // ---- UNDERSTAND ----------------------------------------------------------------------------

    async fn st_understand(
        &self,
        run: &mut AiRun,
        ic: &IntelCtx,
        task: &TaskCtx,
    ) -> IntelResult<StageResult> {
        let records = ic.records()?;
        let cache = self.effect_cache(&run.id);
        // memória relevante (explicável: o que foi usado fica na Run)
        let terms: Vec<String> = run
            .inputs
            .brief_text
            .iter()
            .flat_map(|t| t.split_whitespace().map(str::to_owned))
            .take(60)
            .collect();
        let ret = self.memory.retrieve(&MemoryQuery {
            client_id: run.inputs.client_id.clone(),
            terms,
            limit: 12,
        })?;
        run.memory_used = ret
            .items
            .iter()
            .map(|i| MemoryRef {
                id: i.id.clone(),
                scope: i.scope.as_str().to_owned(),
                content_digest: i.digest(),
            })
            .collect();

        let reinterpret = run.checkpoint["reinterpret"].as_bool().unwrap_or(false);
        let mut spec: Option<DemandSpec> = None;
        if !reinterpret
            && let Some(id) = run
                .inputs
                .demand_spec_id
                .clone()
                .or_else(|| run.demand_spec_id.clone())
        {
            spec = records.latest::<DemandSpec>(KIND_DEMAND, &id)?;
        }
        if spec.is_none() {
            let mut all = Vec::new();
            if let Some(t) = run
                .inputs
                .brief_text
                .clone()
                .filter(|t| !t.trim().is_empty())
            {
                all.push(docs::extract_text(
                    "brief",
                    DocKind::Txt,
                    &t,
                    format!("brief:{}", short(&t)),
                ));
            }
            for path in run.inputs.documents.clone() {
                let p = PathBuf::from(&path);
                all.push(
                    tokio::task::spawn_blocking(move || docs::extract_file(&p))
                        .await
                        .map_err(|e| IntelError::new("INTERNAL", e.to_string()))??,
                );
            }
            if all.is_empty() {
                let d = pending(
                    run,
                    format!("dec-{}-brief-{}", run.id, run.question_rounds),
                    DecisionKind::OpenQuestion,
                    "No briefing was provided. Describe the demand (product, audience, offer, CTA, duration).",
                    vec![DecisionOption::new("answer", "Send the briefing")],
                    json!({"missing": "brief"}),
                    "The run cannot plan without a briefing.",
                    None,
                    RunStage::Understand,
                    None,
                );
                return Ok(StageResult::wait(d));
            }
            let mut note = run.inputs.note.clone().unwrap_or_default();
            for a in &run.inputs.answers {
                note.push_str(&format!("\nUser answer: {a}"));
            }
            let key = format!("llm:{}:demand:{}", run.id, run.question_rounds);
            let hit = super::roles::EffectCache::load(&cache, &key)
                .and_then(|v| serde_json::from_value::<DemandSpec>(v["raw"].clone()).ok());
            let s = if let Some(s) = hit {
                s
            } else {
                let (s, _cached) = crate::demand::interpret(
                    ic,
                    task,
                    &all,
                    &InterpretOptions {
                        user_note: (!note.trim().is_empty()).then_some(note),
                        force: reinterpret,
                    },
                )
                .await?;
                fp!("autonomy_understand_after_provider");
                let meta = RoleMeta {
                    endpoint_id: s.provenance.endpoint_id.clone(),
                    model_id: s.provenance.model_id.clone(),
                    cost_micros: s.provenance.cost_micros,
                    tokens: 0,
                    cache_hit: false,
                    prompt_version: 1,
                    input_digest: s.id.clone(),
                };
                super::roles::EffectCache::save(
                    &cache,
                    &key,
                    &json!({"raw": serde_json::to_value(&s).unwrap_or(Value::Null), "meta": meta}),
                );
                s
            };
            spec = Some(s);
        }
        let spec = spec.ok_or_else(|| IntelError::new("INTERNAL", "no demand spec"))?;
        run.demand_spec_id = Some(spec.id.clone());
        run.demand_spec_version = Some(spec.version);
        Self::set_checkpoint(run, "reinterpret", json!(false));

        // análises determinísticas reaproveitáveis: referência (gramática) e transcrição do bruto
        let mut warnings: Vec<String> = Vec::new();
        for asset in run.inputs.references.clone() {
            if records.latest_json(KIND_REFERENCE, &asset)?.is_some() {
                continue;
            }
            match reference::analyze_reference(
                ic,
                task,
                &asset,
                &ReferenceOptions::default(),
                &|_, _, _| {},
            )
            .await
            {
                Ok(_) => {}
                Err(e) if e.is_cancelled() => return Err(e),
                Err(e) => warnings.push(format!("reference {asset}: {}", e.code)),
            }
        }
        for asset in run.inputs.assets.clone().into_iter().take(6) {
            match transcript::transcribe_asset(ic, task, &TranscribeParams::new(&asset), &|_, _| {})
                .await
            {
                Ok(rec) => {
                    if !rec.from_cache {
                        self.ledger_other(
                            &run.id,
                            &format!("stt:{}:{asset}", run.id),
                            "stt",
                            rec.cost_micros,
                        );
                    }
                }
                Err(e) if e.is_cancelled() => return Err(e),
                Err(e) => warnings.push(format!("transcript {asset}: {}", e.code)),
            }
        }

        // regras explícitas do briefing viram PROPOSTAS de memória (nunca ativas em User/Client)
        if run.checkpoint["mem_proposed"].as_str() != Some(&spec.id) {
            let scope = if run.inputs.client_id.is_some() {
                MemoryScope::Client
            } else {
                MemoryScope::Project
            };
            for it in spec
                .must_avoid
                .iter()
                .chain(spec.constraints.iter())
                .take(8)
            {
                if it.basis != crate::demand::Basis::Explicit {
                    continue;
                }
                let _ = self.memory.propose(
                    MemoryDraft {
                        scope,
                        client_id: run.inputs.client_id.clone(),
                        kind: MemoryKind::Rule,
                        content: format!("Briefing rule: {}", it.text),
                        structured: None,
                        source: MemorySource::Briefing,
                        confidence: 0.8,
                        evidence: it
                            .sources
                            .iter()
                            .take(2)
                            .map(|s| EvidenceRef {
                                kind: "briefing".into(),
                                detail: s.quote.clone(),
                            })
                            .collect(),
                        key: None,
                    },
                    Some(&run.id),
                    run.policy.project_memory_auto_activate,
                );
            }
            Self::set_checkpoint(run, "mem_proposed", json!(spec.id));
        }

        // perguntas abertas / aprovação do DemandSpec
        let qs: Vec<String> = spec
            .open_questions
            .iter()
            .map(|q| q.question.clone())
            .collect();
        let spec_ok = run.checkpoint["spec_approved"].as_u64() == Some(u64::from(spec.version));
        let ask = match run.policy.demand_spec {
            SpecApproval::Always => !spec_ok,
            SpecApproval::RequiredIfQuestions => {
                !qs.is_empty() && run.question_rounds < run.policy.max_question_rounds
            }
            SpecApproval::Auto => false,
        };
        if ask {
            let mut options = vec![DecisionOption::new(
                "approve",
                "Proceed with the current understanding",
            )];
            if !qs.is_empty() {
                options.push(DecisionOption::new("answer", "Answer the open questions"));
            }
            let d = pending(
                run,
                format!(
                    "dec-{}-spec-{}-{}",
                    run.id, spec.version, run.question_rounds
                ),
                DecisionKind::OpenQuestion,
                if qs.is_empty() {
                    "Approve the interpreted demand?"
                } else {
                    "The briefing leaves open questions."
                },
                options,
                json!({"spec": demand_json(&spec), "questions": qs, "warnings": warnings}),
                "Answering refines the DemandSpec; proceeding plans with the stated assumptions.",
                None,
                RunStage::Understand,
                Some("approve"),
            );
            return Ok(StageResult::wait(d));
        }
        let mut r = StageResult::ok(Outcome::Success);
        r.output = json!({"demand_spec": spec.id, "version": spec.version, "warnings": warnings});
        Ok(r)
    }

    // ---- PLAN ----------------------------------------------------------------------------------

    fn planning_context(
        &self,
        run: &AiRun,
        ic: &IntelCtx,
        spec: &DemandSpec,
    ) -> IntelResult<PlanningContext> {
        let records = ic.records()?;
        let inv = self.inventory()?;
        let inventory = json!(
            inv.assets
                .values()
                .map(|a| json!({"id": a.id, "name": a.name, "duration_ms": a.duration_ticks.map(|d| d / super::plan::TICKS_PER_MS),
                    "has_video": a.has_video, "has_audio": a.has_audio, "online": a.online, "image": a.is_image}))
                .collect::<Vec<_>>()
        );
        let mut transcripts = Vec::new();
        for asset in &run.inputs.assets {
            if let Some(v) = records.latest_json(KIND_TRANSCRIPT, asset)? {
                let segs: Vec<String> = v["transcript"]["segments"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(40)
                    .map(|s| {
                        format!(
                            "[{}-{}ms] {}",
                            s["start_us"].as_i64().unwrap_or(0) / 1000,
                            s["end_us"].as_i64().unwrap_or(0) / 1000,
                            s["text"].as_str().unwrap_or("")
                        )
                    })
                    .collect();
                transcripts.push((asset.clone(), segs.join("\n")));
            }
        }
        let reference = run
            .inputs
            .references
            .first()
            .map(|a| records.latest_json(KIND_REFERENCE, a))
            .transpose()?
            .flatten();
        let memory: Vec<String> = run
            .memory_used
            .iter()
            .filter_map(|m| self.memory.get(&m.id).ok().flatten())
            .map(|i| format!("[{}] {}", i.scope.as_str(), i.content))
            .collect();
        let gw = run.policy.allow_gateway && !self.deps.gateways.enabled().is_empty();
        let gen_ok = run.policy.allow_generation && self.deps.generators.available();
        let feedback = run
            .checkpoint
            .get("replan")
            .cloned()
            .filter(Value::is_object);
        Ok(PlanningContext {
            demand: demand_json(spec),
            reference,
            inventory,
            transcripts,
            memory,
            capabilities: json!({"gateway": gw, "generation": gen_ok, "gateway_adapters": self.deps.gateways.ids()}),
            requested: json!({"deliverables": run.inputs.deliverables, "variants": run.inputs.variants,
                              "master_sequence": run.inputs.master_sequence.as_ref().map(|s| format!("seq:{s}")),
                              "budget": run.budget, "replans_used": run.usage.replans}),
            feedback,
        })
    }

    async fn st_plan(
        &self,
        run: &mut AiRun,
        ic: &IntelCtx,
        task: &TaskCtx,
    ) -> IntelResult<StageResult> {
        // replanejamento: contagem e limite (nunca em laço infinito)
        let is_replan = run.production_plan.is_some();
        if is_replan {
            let counted = run.checkpoint["replan_counted"]
                .as_u64()
                .unwrap_or(u64::MAX);
            if counted != u64::from(run.current_attempt) {
                run.usage.replans += 1;
                Self::set_checkpoint(run, "replan_counted", json!(run.current_attempt));
            }
            if run.usage.replans > run.budget.max_replans {
                run.usage.replans = run.budget.max_replans;
                return Ok(StageResult::wait(
                    self.budget_decision(run, BudgetLimit::Replans),
                ));
            }
        }
        let spec = self
            .load_spec(ic, run)?
            .ok_or_else(|| IntelError::new("PLAN_CONTEXT_MISSING", "the demand spec is missing"))?;
        let pc = self.planning_context(run, ic, &spec)?;
        let cache = self.effect_cache(&run.id);
        let version = run.production_plan.as_ref().map_or(1, |p| p.version + 1);
        let pkey = format!("llm:{}:producer:v{version}", run.id);
        let out = roles::run_producer(
            ic,
            task,
            &pc,
            format!("pp_{}_{version}", run.id),
            version,
            run.demand_spec_version,
            Some((&cache, &pkey)),
        )
        .await?;
        fp!("autonomy_plan_after_producer");
        let mut plan = out.value;
        // entregas pedidas precisam constar
        for req in &run.inputs.deliverables {
            if !plan.deliverables.iter().any(|d| d.key == req.key) && run.inputs.variants.is_none()
            {
                return Err(IntelError::new(
                    "PLAN_COVERAGE",
                    format!(
                        "the plan does not include the requested deliverable `{}`",
                        req.key
                    ),
                ));
            }
        }
        // ajuda a plan a respeitar a política de fontes
        for n in &mut plan.asset_needs {
            n.source_priority.retain(|s| match s {
                AcquireSource::Gateway => run.policy.allow_gateway,
                AcquireSource::Generate => run.policy.allow_generation,
                _ => true,
            });
        }
        let mut edits: Vec<EditPlan> = Vec::new();
        for key in plan.execution_order() {
            let ekey = format!("llm:{}:planner:v{version}:{key}", run.id);
            let e = roles::run_planner(
                ic,
                task,
                &pc,
                &plan,
                &key,
                format!("ep_{}_{version}_{key}", run.id),
                version,
                Some((&cache, &ekey)),
            )
            .await?;
            edits.push(e.value);
        }
        // mesmo plano de antes (sem progresso) ⇒ não insiste: pede decisão
        let digest = digest_value(
            &json!({"p": plan.digest(), "e": edits.iter().map(EditPlan::digest).collect::<Vec<_>>()}),
        );
        if is_replan && run.plan_digests.contains(&digest) {
            let d = pending(
                run,
                format!("dec-{}-same-plan-{}", run.id, run.current_attempt),
                DecisionKind::ConflictResolution,
                "Replanning produced the same plan again. How do you want to proceed?",
                vec![
                    DecisionOption::new("stop", "Stop the run"),
                    DecisionOption::new("accept", "Use this plan anyway"),
                ],
                json!({"digest": digest, "replans": run.usage.replans}),
                "The planner is not making progress; continuing may repeat the same problem.",
                None,
                RunStage::Plan,
                Some("stop"),
            );
            return Ok(StageResult::wait(d));
        }
        run.plan_digests.push(digest);
        let records = ic.records()?;
        records.put(KIND_RUN_PLAN, &run.id, plan.version, None, &plan)?;
        for e in &edits {
            records.put(
                KIND_RUN_EDIT_PLAN,
                &format!("{}:{}", run.id, e.deliverable_key),
                e.version,
                Some(&plan.id),
                e,
            )?;
        }
        // propostas de memória do Producer: só PROPOSTAS
        for m in &plan.memory_proposals {
            let scope = MemoryScope::parse(&m.scope).unwrap_or(MemoryScope::Project);
            let _ = self.memory.propose(
                MemoryDraft {
                    scope,
                    client_id: if scope == MemoryScope::Client {
                        run.inputs.client_id.clone()
                    } else {
                        None
                    },
                    kind: MemoryKind::parse(&m.kind),
                    content: m.content.clone(),
                    structured: None,
                    source: MemorySource::Agent,
                    confidence: m.confidence.unwrap_or(0.5),
                    evidence: m
                        .evidence
                        .iter()
                        .map(|e| EvidenceRef {
                            kind: "agent".into(),
                            detail: e.clone(),
                        })
                        .collect(),
                    key: None,
                },
                Some(&run.id),
                run.policy.project_memory_auto_activate,
            );
        }
        let summary = json!({"plan": plan.id, "deliverables": plan.deliverables.len(), "asset_needs": plan.asset_needs.len(), "version": plan.version});
        run.production_plan = Some(plan);
        run.edit_plans = edits;
        run.validation = None;
        run.checkpoint = json!({"retries": run.checkpoint["retries"], "replan_counted": run.checkpoint["replan_counted"], "rejected": run.checkpoint["rejected"], "mem_proposed": run.checkpoint["mem_proposed"], "spec_approved": run.checkpoint["spec_approved"]});
        let mut r = StageResult::ok(Outcome::Success);
        r.events.push(("plan_ready".into(), summary.clone()));
        r.output = summary;
        Ok(r)
    }

    // ---- VALIDATE_PLAN -------------------------------------------------------------------------

    fn resolutions(plan: &ProductionPlan) -> BTreeMap<String, String> {
        plan.asset_needs
            .iter()
            .filter_map(|n| n.resolved_asset_id.clone().map(|a| (n.id.clone(), a)))
            .collect()
    }

    fn compile_units(&self, run: &AiRun, inv: &Inventory, ns: &str) -> IntelResult<Vec<TxUnit>> {
        let plan = run
            .production_plan
            .as_ref()
            .ok_or_else(|| IntelError::new("PLAN_MISSING", "no production plan"))?;
        let res = Self::resolutions(plan);
        let env = CompileEnv {
            run_id: &run.id,
            ns,
            inventory: inv,
            resolutions: &res,
        };
        compile_production(&env, plan, &run.edit_plans)
            .map_err(|e| IntelError::new(e.code, e.message))
    }

    fn replan_feedback(run: &mut AiRun, reason: &str, errors: &[String]) {
        Self::set_checkpoint(
            run,
            "replan",
            json!({"reason": reason, "errors": errors.iter().take(12).collect::<Vec<_>>(),
            "previous_plan_digests": run.plan_digests.len()}),
        );
    }

    async fn st_validate(&self, run: &mut AiRun, ic: &IntelCtx) -> IntelResult<StageResult> {
        let plan = run
            .production_plan
            .clone()
            .ok_or_else(|| IntelError::new("PLAN_MISSING", "no production plan"))?;
        let inv = self.inventory()?;
        let mut errors: Vec<String> = Vec::new();
        for ep in &run.edit_plans {
            for e in validate_edit_plan_semantics(ep, &inv, &plan.asset_needs) {
                errors.push(format!("{}: {}", ep.deliverable_key, e.message));
            }
        }
        let ns = format!("p{}", plan.version);
        let units = if errors.is_empty() {
            match self.compile_units(run, &inv, &ns) {
                Ok(u) => u,
                Err(e) => {
                    errors.push(e.message);
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        let actor = Self::actor(run);
        let mut unit_reports: Vec<UnitValidation> = Vec::new();
        let mut unresolved: BTreeSet<String> = BTreeSet::new();
        let mut warnings: Vec<String> = Vec::new();
        for u in &units {
            let label = format!("AI Run {}: {}", run.id, u.key());
            match self
                .deps
                .engine
                .preview(&actor, &label, Value::Array(u.commands.clone()))
            {
                Ok(pv) => {
                    unit_reports.push(UnitValidation {
                        unit: u.key(),
                        deliverables: u.deliverables.clone(),
                        command_count: u.commands.len(),
                        op_count: pv["op_count"].as_u64().unwrap_or(0) as usize,
                        base_revision: pv["base_revision"].as_u64().unwrap_or(0),
                        diff_digest: pv["diff_digest"].as_str().unwrap_or("").to_owned(),
                        plan_digest: pv["plan_digest"].as_str().unwrap_or("").to_owned(),
                        plan_token: pv["plan_token"].as_str().map(str::to_owned),
                        already_applied: pv["already_applied"].as_bool().unwrap_or(false),
                        destructive_ops: destructive_count(&u.commands),
                        sequences: u.sequences.iter().map(|s| s.1.clone()).collect(),
                    });
                    unresolved.extend(u.unresolved.iter().cloned());
                }
                Err(e) => errors.push(format!("{}: {} ({})", u.key(), e.message, e.code)),
            }
        }
        fp!("autonomy_validate_after_preview");
        let open_needs: Vec<String> = unresolved
            .iter()
            .filter(|n| plan.need(n).is_some_and(|x| x.status == NeedStatus::Open))
            .cloned()
            .collect();
        // estimativa de custo (preço desconhecido ≠ zero)
        let mut acq = 0u64;
        let mut gen_cost = 0u64;
        let mut unknown = false;
        let mut notes = Vec::new();
        for n in plan
            .asset_needs
            .iter()
            .filter(|n| n.status == NeedStatus::Open)
        {
            match n.estimated_cost_micros {
                Some(c)
                    if n.source_priority.contains(&AcquireSource::Generate)
                        && !n.source_priority.contains(&AcquireSource::Gateway) =>
                {
                    gen_cost += c
                }
                Some(c) => acq += c,
                None if n
                    .source_priority
                    .iter()
                    .any(|s| matches!(s, AcquireSource::Gateway | AcquireSource::Generate)) =>
                {
                    unknown = true;
                    notes.push(format!("price of `{}` is unknown", n.id));
                }
                None => {}
            }
        }
        let usage = self.usage_from_ledger(&run.id, &run.usage);
        let cost = CostEstimate {
            llm_micros: usage.cost_micros,
            acquisition_micros: acq,
            generation_micros: gen_cost,
            total_micros: usage.cost_micros + acq + gen_cost,
            has_unknown: unknown || usage.unknown_cost_calls > 0,
            notes,
        };
        let destructive: u32 = unit_reports.iter().map(|u| u.destructive_ops).sum();
        let plan_digest = self.plan_bound(run);
        let diff_digest = digest_value(&json!(
            unit_reports
                .iter()
                .map(|u| u.diff_digest.clone())
                .collect::<Vec<_>>()
        ));
        let target_rev = unit_reports
            .iter()
            .map(|u| u.base_revision)
            .max()
            .unwrap_or(0);
        let plan_gate = run.policy.plan == PlanApproval::Always
            || destructive > run.policy.destructive_threshold;
        let spend_est = acq + gen_cost;
        let spend_gate = spend_est > run.policy.spend_threshold_micros;
        let report = ValidationReport {
            plan_id: plan.id.clone(),
            valid: errors.is_empty(),
            plan_digest: plan_digest.clone(),
            diff_digest,
            target_revision: target_rev,
            command_count: unit_reports.iter().map(|u| u.command_count).sum(),
            sequences_affected: unit_reports
                .iter()
                .flat_map(|u| u.sequences.clone())
                .collect(),
            assets_missing: open_needs.clone(),
            warnings: std::mem::take(&mut warnings),
            errors: errors.clone(),
            estimated_cost: cost.clone(),
            approval_required: plan_gate || spend_gate,
            placeholder_state: unresolved.iter().cloned().collect(),
            units: unit_reports,
        };
        let records = ic.records()?;
        records.put(
            KIND_RUN_VALIDATION,
            &run.id,
            plan.version,
            Some(&plan.id),
            &report,
        )?;
        run.validation = Some(report.clone());

        if !report.valid {
            Self::replan_feedback(run, "validation_failed", &errors);
            let mut r = StageResult::ok(Outcome::InvalidFixable);
            r.output = json!({"errors": errors});
            return Ok(r);
        }
        // orçamento: estimativa + já comprometido vs teto
        if let Some(max) = run.budget.max_cost_micros {
            let committed = self.store.ledger_committed(&run.id).map_err(store_err)?;
            if committed + spend_est > max {
                return Ok(StageResult::wait(
                    self.budget_decision(run, BudgetLimit::Cost),
                ));
            }
        }
        if plan_gate && !Self::approved(run, DecisionKind::PlanApproval, &plan_digest) {
            let d = pending(
                run,
                format!("dec-{}-plan-{}", run.id, short(&plan_digest)),
                DecisionKind::PlanApproval,
                "Approve this production and edit plan before anything is written?",
                vec![
                    DecisionOption::new("approve", "Approve and continue"),
                    DecisionOption::new("change", "Ask for changes (replan)"),
                    DecisionOption::new("reject", "Reject and stop"),
                ],
                json!({"plan": plan, "edit_plans": run.edit_plans, "report": report}),
                "Approving lets the run acquire missing assets and edit the timeline (everything stays editable and undoable).",
                Some(plan_digest.clone()),
                RunStage::ValidatePlan,
                None,
            );
            return Ok(StageResult::wait(d));
        }
        if spend_gate && !Self::approved(run, DecisionKind::SpendApproval, &plan_digest) {
            let d = pending(
                run,
                format!("dec-{}-spend-{}", run.id, short(&plan_digest)),
                DecisionKind::SpendApproval,
                "The estimated spend is above the approval threshold. Approve it?",
                vec![
                    DecisionOption::new("approve", "Approve the spend"),
                    DecisionOption::new("reject", "Reject and stop"),
                ],
                json!({"estimated_cost": cost, "threshold_micros": run.policy.spend_threshold_micros}),
                "Acquired/generated media may cost money up to the estimate (unknown prices are not zero).",
                Some(plan_digest.clone()),
                RunStage::ValidatePlan,
                None,
            );
            return Ok(StageResult::wait(d));
        }
        let outcome = if open_needs.is_empty() {
            Outcome::ValidAssetsOk
        } else {
            Outcome::ValidMissingAssets
        };
        let mut r = StageResult::ok(outcome);
        r.output = json!({"valid": true, "missing": open_needs, "diff_digest": report.diff_digest});
        Ok(r)
    }

    // ---- ACQUIRE -------------------------------------------------------------------------------

    async fn st_acquire(
        &self,
        run: &mut AiRun,
        ic: &IntelCtx,
        task: &TaskCtx,
        flags: &Flags,
    ) -> IntelResult<StageResult> {
        let mut plan = run
            .production_plan
            .clone()
            .ok_or_else(|| IntelError::new("PLAN_MISSING", "no production plan"))?;
        let inv = self.inventory()?;
        let mut used: BTreeSet<String> = plan
            .asset_needs
            .iter()
            .filter_map(|n| n.resolved_asset_id.clone())
            .collect();
        let mut failed_optional: Vec<String> = Vec::new();
        let mut critical: Option<(String, String)> = None;
        let order: Vec<usize> = {
            let mut v: Vec<usize> = (0..plan.asset_needs.len()).collect();
            v.sort_by_key(|i| !plan.asset_needs[*i].required);
            v
        };
        for i in order {
            if plan.asset_needs[i].status != NeedStatus::Open {
                continue;
            }
            let mut need = plan.asset_needs[i].clone();
            match self
                .acquire_need(run, &mut need, &inv, &used, ic, task, flags)
                .await?
            {
                NeedResult::Resolved(asset) => {
                    need.status = NeedStatus::Resolved;
                    need.resolved_asset_id = Some(asset.clone());
                    used.insert(asset);
                    plan.asset_needs[i] = need;
                    run.production_plan = Some(plan.clone());
                    // progresso parcial durável (resume sabe o que já foi resolvido)
                    self.save(
                        run,
                        None,
                        &[(
                            "need_resolved".into(),
                            json!({"need": plan.asset_needs[i].id}),
                        )],
                    )?;
                }
                NeedResult::Wait(d) => {
                    run.production_plan = Some(plan);
                    return Ok(StageResult::wait(*d));
                }
                NeedResult::Unavailable(reason) => {
                    need.status = NeedStatus::Failed;
                    if need.required {
                        critical.get_or_insert((need.id.clone(), reason));
                    } else {
                        failed_optional.push(need.id.clone());
                    }
                    plan.asset_needs[i] = need;
                    run.production_plan = Some(plan.clone());
                }
            }
        }
        run.production_plan = Some(plan.clone());
        if let Some((id, reason)) = critical {
            if run.policy.on_critical_unavailable == CriticalUnavailable::Fail {
                run.error = Some(super::model::RunErrorInfo {
                    kind: super::machine::RunErrorKind::AssetUnavailable,
                    code: "ASSET_UNAVAILABLE".into(),
                    message: format!("required asset `{id}` is unavailable: {reason}"),
                    stage: RunStage::Acquire,
                    recoverable: true,
                });
                return Ok(StageResult::ok(Outcome::Failure));
            }
            // volta a Open para a retomada tentar de novo depois da decisão do usuário
            if let Some(n) = run
                .production_plan
                .as_mut()
                .and_then(|p| p.asset_needs.iter_mut().find(|n| n.id == id))
            {
                n.status = NeedStatus::Open;
            }
            let d = pending(
                run,
                format!("dec-{}-critical-{}-{}", run.id, id, run.current_attempt),
                DecisionKind::ConflictResolution,
                &format!("A required asset (`{id}`) could not be obtained: {reason}"),
                vec![
                    DecisionOption::new(
                        "retry",
                        "Try again (enable a source or add the media first)",
                    ),
                    DecisionOption::new("replan", "Replan without it"),
                    DecisionOption::new("stop", "Stop the run"),
                ],
                json!({"need": id, "reason": reason, "gateway_adapters": self.deps.gateways.ids(),
                       "generation": self.deps.generators.available()}),
                "The editor and everything done so far stay intact.",
                None,
                RunStage::Acquire,
                None,
            );
            return Ok(StageResult::wait(d));
        }
        if !failed_optional.is_empty() {
            Self::replan_feedback(
                run,
                "optional_assets_unavailable",
                &failed_optional
                    .iter()
                    .map(|n| format!("asset need `{n}` could not be acquired"))
                    .collect::<Vec<_>>(),
            );
            let mut r = StageResult::ok(Outcome::PartialFallback);
            r.output = json!({"failed_optional": failed_optional});
            return Ok(r);
        }
        let mut r = StageResult::ok(Outcome::AllReady);
        r.output = json!({"resolved": plan.asset_needs.iter().filter(|n| n.status == NeedStatus::Resolved).count()});
        Ok(r)
    }

    #[allow(clippy::too_many_arguments)]
    async fn acquire_need(
        &self,
        run: &mut AiRun,
        need: &mut AssetNeed,
        inv: &Inventory,
        used: &BTreeSet<String>,
        ic: &IntelCtx,
        task: &TaskCtx,
        flags: &Flags,
    ) -> IntelResult<NeedResult> {
        let _ = (ic, task);
        let order = run.policy.acquire_order.clone();
        let mut notes: Vec<String> = Vec::new();
        for src in order {
            if !need.source_priority.contains(&src)
                && !(src == AcquireSource::Library
                    && need.source_priority.contains(&AcquireSource::Gateway))
            {
                continue;
            }
            match src {
                AcquireSource::Project => {
                    if let Some((asset, _)) =
                        gateway::rank_inventory(need, inv, used).into_iter().next()
                    {
                        self.record_project_provenance(run, need, &asset);
                        return Ok(NeedResult::Resolved(asset));
                    }
                }
                AcquireSource::Library | AcquireSource::Gateway => {
                    if !run.policy.allow_gateway && src == AcquireSource::Gateway {
                        notes.push("gateway disabled by policy".into());
                        continue;
                    }
                    let want_lib = src == AcquireSource::Library;
                    let adapters: Vec<_> = self
                        .deps
                        .gateways
                        .enabled()
                        .into_iter()
                        .filter(|a| (a.kind() == GatewayKind::LocalLibrary) == want_lib)
                        .collect();
                    if adapters.is_empty() {
                        notes.push(format!(
                            "no {} adapter is enabled",
                            if want_lib { "library" } else { "gateway" }
                        ));
                        continue;
                    }
                    for a in adapters {
                        match self.try_adapter(run, need, &*a, flags).await? {
                            NeedResult::Unavailable(r) => notes.push(format!("{}: {r}", a.id())),
                            other => return Ok(other),
                        }
                    }
                }
                AcquireSource::Generate => {
                    if !run.policy.allow_generation {
                        notes.push("generation disabled by policy".into());
                        continue;
                    }
                    match self.try_generate(run, need, flags).await? {
                        NeedResult::Unavailable(r) => notes.push(format!("generation: {r}")),
                        other => return Ok(other),
                    }
                }
            }
        }
        Ok(NeedResult::Unavailable(if notes.is_empty() {
            "no source could serve it".into()
        } else {
            notes.join("; ")
        }))
    }

    fn record_project_provenance(&self, run: &AiRun, need: &AssetNeed, asset: &str) {
        if self.store.get_provenance(asset).ok().flatten().is_some() {
            return;
        }
        let j = gateway::provenance_json(
            "project",
            &run.id,
            &need.id,
            "project",
            None,
            gateway::LicenseStatus::UserProvided,
            None,
            "",
            Some(0),
            None,
            json!({}),
        );
        let _ = self
            .store
            .put_provenance(asset, Some(&run.id), "project", "", &j, now_ms());
    }

    async fn try_adapter(
        &self,
        run: &mut AiRun,
        need: &AssetNeed,
        adapter: &dyn gateway::AssetGatewayAdapter,
        flags: &Flags,
    ) -> IntelResult<NeedResult> {
        let req = SearchRequest::for_need(need);
        let mut cands = None;
        let mut last = String::new();
        for attempt in 0..3u32 {
            if flags.cancel.is_cancelled() {
                return Err(IntelError::cancelled());
            }
            match adapter.search(&req, &flags.cancel).await {
                Ok(c) => {
                    cands = Some(c);
                    break;
                }
                Err(e) if e.retryable && attempt < 2 => {
                    last = e.message;
                    tokio::time::sleep(Duration::from_millis(100 * (1 << attempt))).await;
                }
                Err(e) => {
                    return Ok(NeedResult::Unavailable(format!(
                        "{}: {}",
                        e.code, e.message
                    )));
                }
            }
        }
        let Some(mut cands) = cands else {
            return Ok(NeedResult::Unavailable(last));
        };
        for c in &mut cands {
            gateway::rank_candidate(c, need);
        }
        cands.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(core::cmp::Ordering::Equal)
                .then(a.id.cmp(&b.id))
        });
        let rejected: BTreeSet<String> = run.checkpoint["rejected"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        for c in cands {
            let key = format!("{}:{}", adapter.id(), c.id);
            if rejected.contains(&key) {
                continue;
            }
            let verdict = gateway::license_verdict(c.license, &run.policy);
            if verdict == LicenseVerdict::Reject {
                continue;
            }
            let bound = digest_value(&json!(key));
            let paid = adapter.is_paid() || c.price_micros.is_none_or(|p| p > 0);
            let est = c.price_micros;
            if verdict == LicenseVerdict::NeedsApproval
                && !Self::approved(run, DecisionKind::AssetApproval, &bound)
            {
                let d = pending(
                    run,
                    format!("dec-{}-asset-{}", run.id, short(&key)),
                    DecisionKind::AssetApproval,
                    &format!(
                        "Use `{}` from `{}`? Its license is {:?}.",
                        c.title,
                        adapter.id(),
                        c.license
                    ),
                    vec![
                        DecisionOption::new("approve", "Use it"),
                        DecisionOption::new("reject", "Skip this asset"),
                    ],
                    json!({"candidate": c, "need": need.id}),
                    "The asset becomes part of your project with this license status recorded.",
                    Some(bound.clone()),
                    RunStage::Acquire,
                    Some("reject"),
                );
                return Ok(NeedResult::Wait(Box::new(d)));
            }
            if paid
                && (est.is_none() || est.unwrap_or(0) > run.policy.spend_threshold_micros)
                && !Self::approved(run, DecisionKind::SpendApproval, &bound)
            {
                let d = pending(
                    run,
                    format!("dec-{}-spendasset-{}", run.id, short(&key)),
                    DecisionKind::SpendApproval,
                    &format!(
                        "`{}` from `{}` is paid ({}). Approve?",
                        c.title,
                        adapter.id(),
                        est.map_or("price unknown".to_owned(), |p| format!("{p} micros"))
                    ),
                    vec![
                        DecisionOption::new("approve", "Approve the spend"),
                        DecisionOption::new("reject", "Skip this asset"),
                    ],
                    json!({"candidate": c, "need": need.id}),
                    "Unknown prices are not treated as zero.",
                    Some(bound.clone()),
                    RunStage::Acquire,
                    Some("reject"),
                );
                return Ok(NeedResult::Wait(Box::new(d)));
            }
            return self
                .fetch_and_import(run, need, adapter, &c, &key, flags)
                .await;
        }
        Ok(NeedResult::Unavailable("no acceptable candidate".into()))
    }

    async fn fetch_and_import(
        &self,
        run: &mut AiRun,
        need: &AssetNeed,
        adapter: &dyn gateway::AssetGatewayAdapter,
        c: &Candidate,
        cand_key: &str,
        flags: &Flags,
    ) -> IntelResult<NeedResult> {
        let ekey = format!("gw:{}:{}:{}", run.id, need.id, short(cand_key));
        let est = c.price_micros.unwrap_or(0);
        let reserved = self
            .store
            .ledger_reserve(
                &run.id,
                &ekey,
                est,
                run.budget.max_cost_micros,
                &json!({"kind": "gateway"}),
                now_ms(),
            )
            .map_err(store_err)?;
        if !reserved {
            return Ok(NeedResult::Wait(Box::new(
                self.budget_decision(run, BudgetLimit::Cost),
            )));
        }
        let claim = self
            .store
            .claim_effect(
                &ekey,
                &run.id,
                "gateway",
                &json!({"adapter": adapter.id(), "candidate": c.id}),
                now_ms(),
            )
            .map_err(store_err)?;
        let (state, prior) = match claim {
            Claim::New(_) => ("intent".to_owned(), Value::Null),
            Claim::Existing(e) => {
                if e.state == "done"
                    && let Some(a) = e.external_id
                {
                    return Ok(NeedResult::Resolved(a));
                }
                if e.state == "failed" {
                    let _ = self.store.ledger_release(&run.id, &ekey, now_ms());
                    return Ok(NeedResult::Unavailable("a previous attempt failed".into()));
                }
                (e.state, e.json)
            }
        };
        let _ = std::fs::create_dir_all(&self.staging);
        let staged = self.staging.join(format!("{}.bin", short(&ekey)));
        // já baixado e confere: não baixa de novo (resume depois do download)
        let have = state == "submitted"
            && staged.exists()
            && prior["sha"]
                .as_str()
                .is_some_and(|sha| gateway_sha(&staged).as_deref() == Some(sha));
        let (sha, bytes) = if have {
            (
                prior["sha"].as_str().unwrap_or("").to_owned(),
                prior["bytes"].as_u64().unwrap_or(0),
            )
        } else {
            let mut last = None;
            let mut got = None;
            for attempt in 0..3u32 {
                if flags.cancel.is_cancelled() {
                    let _ = self.store.ledger_release(&run.id, &ekey, now_ms());
                    return Err(IntelError::cancelled());
                }
                match adapter.fetch(c, &staged, &flags.cancel).await {
                    Ok(f) => {
                        got = Some(f);
                        break;
                    }
                    Err(e) if e.retryable && attempt < 2 => {
                        last = Some(e.message);
                        tokio::time::sleep(Duration::from_millis(100 * (1 << attempt))).await;
                    }
                    Err(e) => {
                        let _ = self.store.update_effect(
                            &ekey,
                            "failed",
                            None,
                            Some(&json!({"error": e.code})),
                            now_ms(),
                        );
                        let _ = self.store.ledger_release(&run.id, &ekey, now_ms());
                        return Ok(NeedResult::Unavailable(format!(
                            "{}: {}",
                            e.code, e.message
                        )));
                    }
                }
            }
            let Some(f) = got else {
                let _ = self.store.ledger_release(&run.id, &ekey, now_ms());
                return Ok(NeedResult::Unavailable(
                    last.unwrap_or_else(|| "download failed".into()),
                ));
            };
            self.store
                .update_effect(
                    &ekey,
                    "submitted",
                    Some(&f.sha256),
                    Some(&json!({"adapter": adapter.id(), "candidate": c.id, "sha": f.sha256, "bytes": f.bytes, "path": staged.display().to_string()})),
                    now_ms(),
                )
                .map_err(store_err)?;
            (f.sha256, f.bytes)
        };
        fp!("autonomy_acquire_after_download");
        let durable = self.make_durable(&staged, &sha);
        let asset = match self.import_staged(&durable, &ekey, flags).await {
            Ok(a) => a,
            Err(e) if e.is_cancelled() => {
                let _ = self.store.ledger_release(&run.id, &ekey, now_ms());
                return Err(e);
            }
            Err(e) => {
                let _ = self.store.update_effect(
                    &ekey,
                    "failed",
                    None,
                    Some(&json!({"error": e.code})),
                    now_ms(),
                );
                let _ = self.store.ledger_release(&run.id, &ekey, now_ms());
                return Ok(NeedResult::Unavailable(format!(
                    "import failed: {}",
                    e.message
                )));
            }
        };
        let kind = if adapter.kind() == GatewayKind::LocalLibrary {
            "library"
        } else {
            "downloaded"
        };
        if self
            .store
            .get_provenance(&asset)
            .map_err(store_err)?
            .is_none()
        {
            let approval = run
                .approvals
                .iter()
                .rev()
                .find(|a| {
                    matches!(
                        a.kind,
                        DecisionKind::AssetApproval | DecisionKind::SpendApproval
                    ) && a.bound_digest.as_deref() == Some(&digest_value(&json!(cand_key)))
                })
                .map(|a| a.decision_id.clone());
            let j = gateway::provenance_json(
                kind,
                &run.id,
                &need.id,
                &adapter.id(),
                c.source_uri.as_deref(),
                c.license,
                c.license_text.as_deref(),
                &sha,
                c.price_micros,
                approval.as_deref(),
                json!({"title": c.title, "score": c.score, "score_components": c.score_components, "bytes": bytes}),
            );
            self.store
                .put_provenance(&asset, Some(&run.id), kind, &sha, &j, now_ms())
                .map_err(store_err)?;
        }
        self.store
            .update_effect(&ekey, "done", Some(&asset), None, now_ms())
            .map_err(store_err)?;
        let _ = self.store.ledger_settle(
            &run.id,
            &ekey,
            est,
            &json!({"kind": "gateway", "unknown": c.price_micros.is_none(), "tokens": 0}),
            now_ms(),
        );
        self.emit(
            &run.id,
            "asset_acquired",
            json!({"need": need.id, "asset": asset, "adapter": adapter.id(), "kind": kind}),
        );
        Ok(NeedResult::Resolved(asset))
    }

    /// Move o arquivo adquirido para a pasta de mídia **durável** do projeto (o catálogo guarda
    /// o caminho; o cache é descartável, então o original não pode ficar nele). Idempotente.
    fn make_durable(&self, from: &std::path::Path, sha: &str) -> PathBuf {
        let _ = std::fs::create_dir_all(&self.media_dir);
        let name = sha.trim_start_matches("sha256:");
        let to = self.media_dir.join(format!("{name}.bin"));
        if to.exists() {
            let _ = std::fs::remove_file(from);
            return to;
        }
        if std::fs::rename(from, &to).is_err() {
            if std::fs::copy(from, &to).is_ok() {
                let _ = std::fs::remove_file(from);
            } else {
                return from.to_path_buf();
            }
        }
        to
    }

    /// Importa o arquivo em staging pelo sistema de assets (atômico) e devolve o `asset_id`.
    async fn import_staged(
        &self,
        path: &std::path::Path,
        parent_key: &str,
        flags: &Flags,
    ) -> IntelResult<String> {
        let ikey = format!("imp:{parent_key}");
        let _ = self
            .store
            .claim_effect(&ikey, "import", "import", &Value::Null, now_ms());
        let ticket = self.deps.engine.import_begin(path)?;
        let _ = self
            .store
            .update_effect(&ikey, "submitted", Some(&ticket), None, now_ms());
        let t0 = std::time::Instant::now();
        loop {
            if flags.cancel.is_cancelled() {
                let _ = self.deps.engine.import_cancel(&ticket);
                return Err(IntelError::cancelled());
            }
            let row = self.deps.engine.import_poll(&ticket)?;
            match row["state"].as_str() {
                Some("finalized") => {
                    let id = row["asset_id"].as_str().map(str::to_owned).ok_or_else(|| {
                        IntelError::new("ASSET_IMPORT", "the import finished without an asset id")
                    })?;
                    let _ = self
                        .store
                        .update_effect(&ikey, "done", Some(&id), None, now_ms());
                    return Ok(id);
                }
                Some("failed" | "cancelled" | "interrupted") => {
                    return Err(IntelError::new(
                        "ASSET_IMPORT",
                        format!(
                            "the import ended as {}",
                            row["state"].as_str().unwrap_or("?")
                        ),
                    ));
                }
                _ => {}
            }
            if t0.elapsed() > Duration::from_secs(180) {
                let _ = self.deps.engine.import_cancel(&ticket);
                return Err(IntelError::new("ASSET_IMPORT", "the import timed out"));
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }

    async fn try_generate(
        &self,
        run: &mut AiRun,
        need: &AssetNeed,
        flags: &Flags,
    ) -> IntelResult<NeedResult> {
        let kind = match need.kind {
            AssetKind::Video => GenKind::Video,
            AssetKind::Image => GenKind::Image,
            AssetKind::Audio => GenKind::Tts,
        };
        let Some(provider) = self.deps.generators.for_kind(kind) else {
            return Ok(NeedResult::Unavailable(
                "no generation provider is available".into(),
            ));
        };
        let prompt = format!("{}. {}", need.purpose, need.description);
        let model = "default".to_owned();
        let version = run.production_plan.as_ref().map_or(1, |p| p.version);
        let key = generation::idempotency_key(&run.id, &need.id, &prompt, &model, version);
        let req = GenRequest {
            kind,
            purpose: need.purpose.clone(),
            prompt: prompt.clone(),
            reference_assets: run.inputs.assets.iter().take(3).cloned().collect(),
            width: None,
            height: None,
            duration_ms: need.target_duration_ms,
            model,
            safety: need.constraints.clone(),
            idempotency_key: key.clone(),
            params: json!({"need": need.id}),
        };
        let est = provider.estimate(&req);
        let bound = digest_value(&json!(key));
        // limite de gerações (orçamento)
        let used = self.usage_from_ledger(&run.id, &run.usage).generations;
        if run.budget.max_generations.is_some_and(|m| used >= m)
            && self.store.get_effect(&key).map_err(store_err)?.is_none()
        {
            return Ok(NeedResult::Wait(Box::new(
                self.budget_decision(run, BudgetLimit::Generations),
            )));
        }
        if (run.policy.generation_requires_approval || est.is_none())
            && !Self::approved(run, DecisionKind::GenerationApproval, &bound)
        {
            let d = pending(
                run,
                format!("dec-{}-gen-{}", run.id, short(&key)),
                DecisionKind::GenerationApproval,
                &format!(
                    "Generate media for `{}`? {}",
                    need.id,
                    est.map_or("The price is unknown.".to_owned(), |p| format!(
                        "Estimated cost: {p} micros."
                    ))
                ),
                vec![
                    DecisionOption::new("approve", "Generate"),
                    DecisionOption::new("reject", "Do not generate"),
                ],
                json!({"need": need.id, "prompt": prompt, "provider": provider.id(), "estimated_micros": est}),
                "Generation is paid and the result is recorded with its provenance (model, prompt, parameters).",
                Some(bound.clone()),
                RunStage::Acquire,
                Some("reject"),
            );
            return Ok(NeedResult::Wait(Box::new(d)));
        }
        let reserve = est.unwrap_or(0);
        if !self
            .store
            .ledger_reserve(
                &run.id,
                &key,
                reserve,
                run.budget.max_cost_micros,
                &json!({"kind": "generation"}),
                now_ms(),
            )
            .map_err(store_err)?
        {
            return Ok(NeedResult::Wait(Box::new(
                self.budget_decision(run, BudgetLimit::Cost),
            )));
        }
        let claim = self
            .store
            .claim_effect(
                &key,
                &run.id,
                "generation",
                &json!({"provider": provider.id(), "need": need.id}),
                now_ms(),
            )
            .map_err(store_err)?;
        let (state, job_prior) = match claim {
            Claim::New(_) => ("intent".to_owned(), None),
            Claim::Existing(e) => {
                if e.state == "done"
                    && let Some(a) = e.external_id
                {
                    return Ok(NeedResult::Resolved(a));
                }
                if e.state == "failed" {
                    let _ = self.store.ledger_release(&run.id, &key, now_ms());
                    return Ok(NeedResult::Unavailable(
                        "a previous generation failed".into(),
                    ));
                }
                (e.state, e.external_id)
            }
        };
        // NUNCA submete de novo sem confirmar que o job anterior não existe
        let job = match (state.as_str(), job_prior) {
            ("submitted", Some(j)) => j,
            _ => match provider.lookup(&key).await {
                Ok(Some(j)) => j,
                Ok(None) => match provider.submit(&req).await {
                    Ok(j) => j,
                    Err(e) => {
                        let _ = self.store.update_effect(
                            &key,
                            "failed",
                            None,
                            Some(&json!({"error": e.code, "provider": provider.id()})),
                            now_ms(),
                        );
                        let _ = self.store.ledger_release(&run.id, &key, now_ms());
                        return Ok(NeedResult::Unavailable(format!(
                            "{}: {}",
                            e.code, e.message
                        )));
                    }
                },
                Err(e) => {
                    let _ = self.store.ledger_release(&run.id, &key, now_ms());
                    return Ok(NeedResult::Unavailable(format!(
                        "{}: {}",
                        e.code, e.message
                    )));
                }
            },
        };
        self.store
            .update_effect(
                &key,
                "submitted",
                Some(&job),
                Some(&json!({"provider": provider.id(), "need": need.id, "job": job})),
                now_ms(),
            )
            .map_err(store_err)?;
        fp!("autonomy_generation_after_submit");
        let _ = std::fs::create_dir_all(&self.staging);
        let out = self.staging.join(format!("{}.gen", short(&key)));
        let t0 = std::time::Instant::now();
        let produced = loop {
            if flags.cancel.is_cancelled() {
                let _ = provider.cancel(&job).await;
                let _ = self.store.ledger_release(&run.id, &key, now_ms());
                return Err(IntelError::cancelled());
            }
            match provider.poll(&job, &out, &flags.cancel).await {
                Ok(GenStatus::Done { path, .. }) => break path,
                Ok(GenStatus::Pending | GenStatus::Running) => {}
                Ok(GenStatus::Failed { message, .. }) => {
                    let _ = self.store.update_effect(
                        &key,
                        "failed",
                        None,
                        Some(&json!({"error": "GEN_FAILED", "provider": provider.id()})),
                        now_ms(),
                    );
                    let _ = self.store.ledger_release(&run.id, &key, now_ms());
                    return Ok(NeedResult::Unavailable(message));
                }
                Err(e) if e.code == "CANCELLED" => return Err(IntelError::cancelled()),
                Err(e) => {
                    let _ = self.store.ledger_release(&run.id, &key, now_ms());
                    return Ok(NeedResult::Unavailable(format!(
                        "{}: {}",
                        e.code, e.message
                    )));
                }
            }
            if t0.elapsed() > Duration::from_secs(900) {
                let _ = self.store.ledger_release(&run.id, &key, now_ms());
                return Ok(NeedResult::Unavailable("generation timed out".into()));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let len = std::fs::metadata(&produced).map_or(0, |m| m.len());
        if len == 0 {
            let _ = self.store.update_effect(
                &key,
                "failed",
                None,
                Some(&json!({"error": "GEN_EMPTY_OUTPUT", "provider": provider.id()})),
                now_ms(),
            );
            let _ = self.store.ledger_release(&run.id, &key, now_ms());
            let _ = std::fs::remove_file(&produced);
            return Ok(NeedResult::Unavailable(
                "the generation produced an invalid (empty) output".into(),
            ));
        }
        let sha = gateway_sha(&produced).unwrap_or_default();
        let durable = self.make_durable(&produced, &sha);
        let asset = match self.import_staged(&durable, &key, flags).await {
            Ok(a) => a,
            Err(e) if e.is_cancelled() => return Err(e),
            Err(e) => {
                let _ = self.store.update_effect(
                    &key,
                    "failed",
                    None,
                    Some(&json!({"error": e.code, "provider": provider.id()})),
                    now_ms(),
                );
                let _ = self.store.ledger_release(&run.id, &key, now_ms());
                return Ok(NeedResult::Unavailable(format!(
                    "import failed: {}",
                    e.message
                )));
            }
        };
        if self
            .store
            .get_provenance(&asset)
            .map_err(store_err)?
            .is_none()
        {
            let approval = run
                .approvals
                .iter()
                .rev()
                .find(|a| {
                    a.kind == DecisionKind::GenerationApproval
                        && a.bound_digest.as_deref() == Some(&bound)
                })
                .map(|a| a.decision_id.clone());
            let j = gateway::provenance_json(
                "generated",
                &run.id,
                &need.id,
                &provider.id(),
                None,
                gateway::LicenseStatus::Generated,
                None,
                &sha,
                est,
                approval.as_deref(),
                json!({"provider": provider.id(), "model": req.model, "prompt_hash": digest_value(&json!(prompt)), "prompt": prompt,
                       "params": req.params, "parent_assets": req.reference_assets, "job": job, "version": version}),
            );
            self.store
                .put_provenance(&asset, Some(&run.id), "generated", &sha, &j, now_ms())
                .map_err(store_err)?;
        }
        self.store
            .update_effect(&key, "done", Some(&asset), None, now_ms())
            .map_err(store_err)?;
        let _ = self.store.ledger_settle(
            &run.id,
            &key,
            est.unwrap_or(0),
            &json!({"kind": "generation", "unknown": est.is_none(), "tokens": 0}),
            now_ms(),
        );
        self.emit(
            &run.id,
            "asset_generated",
            json!({"need": need.id, "asset": asset, "provider": provider.id()}),
        );
        Ok(NeedResult::Resolved(asset))
    }

    // ---- EDIT ----------------------------------------------------------------------------------

    /// Relê o estado da Run no store: cancelado/pausado ⇒ **nada** é aplicado (sem apply tardio).
    fn ensure_running(&self, run_id: &str) -> IntelResult<()> {
        let fresh = self.load(run_id)?;
        if fresh.status != super::machine::RunStatus::Running {
            return Err(IntelError::cancelled());
        }
        Ok(())
    }

    async fn st_edit(
        &self,
        run: &mut AiRun,
        ic: &IntelCtx,
        flags: &Flags,
    ) -> IntelResult<StageResult> {
        let _ = ic;
        let report = run.validation.clone().filter(|v| v.valid).ok_or_else(|| {
            IntelError::new(
                "PLAN_NOT_VALIDATED",
                "no write is allowed before a validated plan",
            )
        })?;
        let plan = run
            .production_plan
            .clone()
            .ok_or_else(|| IntelError::new("PLAN_MISSING", "no production plan"))?;
        // as aprovações exigidas continuam valendo para o plano ATUAL
        let bound = self.plan_bound(run);
        if report.plan_digest != bound {
            return Ok(StageResult::ok(Outcome::Drift));
        }
        if (run.policy.plan == PlanApproval::Always
            || report.units.iter().map(|u| u.destructive_ops).sum::<u32>()
                > run.policy.destructive_threshold)
            && !Self::approved(run, DecisionKind::PlanApproval, &bound)
        {
            return Ok(StageResult::ok(Outcome::Drift));
        }
        let inv = self.inventory()?;
        let units = match self.compile_units(run, &inv, &format!("p{}", plan.version)) {
            Ok(u) => u,
            Err(e) => {
                Self::replan_feedback(run, "compile_failed", &[e.message]);
                return Ok(StageResult::ok(Outcome::Conflict));
            }
        };
        let actor = Self::actor(run);
        let applied_units: BTreeSet<String> = run.checkpoint["applied_units"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        let mut done_now: Vec<String> = applied_units.iter().cloned().collect();
        for u in &units {
            let ukey = u.key();
            if applied_units.contains(&ukey) {
                continue;
            }
            self.ensure_running(&run.id)?;
            if flags.cancel.is_cancelled() {
                return Err(IntelError::cancelled());
            }
            let label = format!("AI Run {}: {ukey}", run.id);
            // refaz o preview (os tokens vivem na memória do engine) e confere o digest validado
            let pv =
                match self
                    .deps
                    .engine
                    .preview(&actor, &label, Value::Array(u.commands.clone()))
                {
                    Ok(p) => p,
                    Err(e) => {
                        Self::replan_feedback(
                            run,
                            "preview_failed_at_edit",
                            &[format!("{}: {}", e.message, e.code)],
                        );
                        return Ok(StageResult::ok(if e.code == "PLAN_STATE_CHANGED" {
                            Outcome::Drift
                        } else {
                            Outcome::Conflict
                        }));
                    }
                };
            let already = pv["already_applied"].as_bool().unwrap_or(false);
            if !already {
                let validated = report.units.iter().find(|v| v.unit == ukey);
                let digest = pv["diff_digest"].as_str().unwrap_or("");
                if validated.is_none_or(|v| v.diff_digest != digest) {
                    // o documento mudou (edição manual?) desde a validação: revalida, não aplica
                    return Ok(StageResult::ok(Outcome::Drift));
                }
                let token = pv["plan_token"]
                    .as_str()
                    .ok_or_else(|| {
                        IntelError::new("PLAN_TOKEN_MISSING", "preview did not return a token")
                    })?
                    .to_owned();
                fp!("autonomy_edit_before_apply");
                self.ensure_running(&run.id)?;
                let reply = match self.deps.engine.apply(&actor, &token) {
                    Ok(r) => r,
                    Err(e) => {
                        Self::replan_feedback(
                            run,
                            "apply_failed",
                            &[format!("{}: {}", e.message, e.code)],
                        );
                        return Ok(StageResult::ok(if e.code == "PLAN_STATE_CHANGED" {
                            Outcome::Drift
                        } else {
                            Outcome::Conflict
                        }));
                    }
                };
                fp!("autonomy_edit_after_apply");
                let rev = reply["revision"].as_u64().unwrap_or(0);
                let entry = self.last_entry_of(&run.actor_id());
                run.applied.push(AppliedRef {
                    stage: RunStage::Edit,
                    deliverable: ukey.clone(),
                    entry_id: entry,
                    revision_before: rev.saturating_sub(1),
                    revision_after: rev,
                    operation_namespace: format!("{}:p{}", run.id, plan.version),
                    cycle: 0,
                });
            } else if !run
                .applied
                .iter()
                .any(|a| a.deliverable == ukey && a.stage == RunStage::Edit)
            {
                // aplicado antes do crash, antes do checkpoint: reconcilia pelo histórico
                let entry = self.last_entry_of(&run.actor_id());
                run.applied.push(AppliedRef {
                    stage: RunStage::Edit,
                    deliverable: ukey.clone(),
                    entry_id: entry,
                    revision_before: 0,
                    revision_after: self.deps.engine.revision().unwrap_or(0),
                    operation_namespace: format!("{}:p{}", run.id, plan.version),
                    cycle: 0,
                });
            }
            done_now.push(ukey.clone());
            Self::set_checkpoint(run, "applied_units", json!(done_now));
            // checkpoint durável antes do próximo lote (resume sabe o que já foi aplicado)
            self.save(run, None, &[("edit_applied".into(), json!({"unit": ukey}))])?;
        }
        let fresh: Vec<ProducedSequence> = units
            .iter()
            .flat_map(|u| {
                u.sequences.iter().map(|(d, s)| ProducedSequence {
                    deliverable: d.clone(),
                    sequence_id: s.clone(),
                    role: plan
                        .deliverable(d)
                        .map_or("deliverable", |x| match x.sequence_strategy {
                            super::plan::SequenceStrategy::Standalone => "standalone",
                            super::plan::SequenceStrategy::HookPlusMaster => "hook",
                            super::plan::SequenceStrategy::SharedMaster => "variant",
                            super::plan::SequenceStrategy::FormatVariant => "format_variant",
                        })
                        .to_owned(),
                })
            })
            .collect();
        // replanejar depois de aplicar: as sequences anteriores ficam (editáveis) marcadas como
        // substituídas — nunca são apagadas pela Run
        let mut merged: Vec<ProducedSequence> = run
            .sequences
            .iter()
            .filter(|old| !fresh.iter().any(|n| n.sequence_id == old.sequence_id))
            .map(|old| ProducedSequence {
                role: "superseded".into(),
                ..old.clone()
            })
            .collect();
        merged.extend(fresh);
        run.sequences = merged;
        let mut r = StageResult::ok(Outcome::ApplyOk);
        r.events.push((
            "edit_applied".into(),
            json!({"units": done_now, "sequences": run.sequences}),
        ));
        r.output = json!({"applied": done_now});
        Ok(r)
    }

    /// Última entrada de histórico do ator desta Run (reconciliação/undo seletivo).
    fn last_entry_of(&self, actor_id: &str) -> u64 {
        self.deps
            .engine
            .read("history.list", json!({}))
            .ok()
            .and_then(|h| {
                h["entries"].as_array().and_then(|a| {
                    a.iter()
                        .rev()
                        .find(|e| e["actor"]["id"] == actor_id)
                        .and_then(|e| e["id"].as_u64())
                })
            })
            .unwrap_or(0)
    }

    // ---- REVIEW --------------------------------------------------------------------------------

    fn review_decisions(run: &AiRun) -> (BTreeSet<String>, BTreeSet<String>) {
        let get = |k: &str| -> BTreeSet<String> {
            run.checkpoint["review_decisions"][k]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        };
        (get("ignored"), get("locked"))
    }

    fn timeline_digest(seq: &Value) -> Value {
        let clips: Vec<Value> = seq["clips"]
            .as_object()
            .into_iter()
            .flatten()
            .take(150)
            .map(|(id, c)| {
                json!({"id": id, "track": c["track"], "start": c["start"], "duration": c["duration"], "name": c["name"],
                       "type": c["content"]["type"], "text": c["content"]["text"], "asset": c["content"]["asset"]})
            })
            .collect();
        json!({"header": seq["header"], "tracks": seq["tracks"], "clips": clips})
    }

    async fn st_review(
        &self,
        run: &mut AiRun,
        ic: &IntelCtx,
        task: &TaskCtx,
    ) -> IntelResult<StageResult> {
        let plan = run
            .production_plan
            .clone()
            .ok_or_else(|| IntelError::new("PLAN_MISSING", "no production plan"))?;
        let spec = self.load_spec(ic, run)?;
        let facts = spec
            .as_ref()
            .map(|s| brief_facts(s, run))
            .unwrap_or_default();
        let inv = self.inventory()?;
        let mut seqs: Vec<(String, Value)> = Vec::new();
        for p in &run.sequences {
            if let Ok(s) = self
                .deps
                .engine
                .read("sequence.get", json!({"sequence": p.sequence_id}))
            {
                seqs.push((p.deliverable.clone(), s));
            }
        }
        let cycle = run.usage.review_loops;
        let mut findings = critic::deterministic_checks(&CheckInput {
            sequences: &seqs,
            edit_plans: &run.edit_plans,
            brief: &facts,
            inventory: &inv,
            cycle,
        });
        // proveniência obrigatória: asset adquirido/gerado sem registro bloqueia o uso automático
        for n in plan
            .asset_needs
            .iter()
            .filter(|n| n.status == NeedStatus::Resolved)
        {
            if let Some(a) = &n.resolved_asset_id
                && self.store.get_provenance(a).map_err(store_err)?.is_none()
            {
                findings.push(critic::Finding {
                    id: format!("prov:{a}"),
                    key: format!("prov:{a}"),
                    severity: critic::Severity::Blocker,
                    category: critic::Category::Technical,
                    source: critic::FindingSource::Deterministic,
                    at: None,
                    evidence: vec![critic::Evidence {
                        kind: "asset".into(),
                        detail: json!({"asset": a, "need": n.id}),
                    }],
                    expected: "every acquired/generated asset has a provenance record".into(),
                    observed: format!("asset `{a}` has none"),
                    suggested_fix: None,
                    confidence: 1.0,
                    status: FindingStatus::Open,
                    needs_replan: true,
                    deliverable: None,
                });
            }
        }
        // semântico (LLM) — só se houver brain; falha do provider não derruba o review
        let cache = self.effect_cache(&run.id);
        let mut prov = json!({"rubric_version": critic::RUBRIC_VERSION, "revision": self.deps.engine.revision().unwrap_or(0)});
        let det_json = json!(
            findings
                .iter()
                .map(|f| json!({"key": f.key, "observed": f.observed}))
                .collect::<Vec<_>>()
        );
        let (ign, lck) = Self::review_decisions(run);
        for (dk, seq) in &seqs {
            let cc = CriticContext {
                demand: spec.as_ref().map(demand_json).unwrap_or(Value::Null),
                plan: serde_json::to_value(
                    run.edit_plans.iter().find(|e| &e.deliverable_key == dk),
                )
                .unwrap_or(Value::Null),
                timeline_digest: Self::timeline_digest(seq),
                transcript: None,
                deterministic: det_json.clone(),
                decisions: json!({"ignored": ign, "locked": lck}),
            };
            let key = format!("llm:{}:critic:{cycle}:{dk}", run.id);
            match roles::run_semantic_critic(ic, task, &cc, dk, Some((&cache, &key))).await {
                Ok(out) => {
                    prov["semantic_model"] = json!(out.meta.model_id);
                    findings.extend(out.value);
                }
                Err(e) if e.is_cancelled() => return Err(e),
                Err(e) => {
                    prov["semantic_error"] = json!(e.code);
                }
            }
        }
        critic::apply_decisions(&mut findings, &ign, &lck);
        let revision = self.deps.engine.revision().unwrap_or(0);
        let review: Review =
            critic::make_review(&run.id, &plan.id, revision, cycle, findings, prov);
        let records = ic.records()?;
        records.put(
            KIND_RUN_REVIEW,
            &format!("{}:{cycle}", run.id),
            1,
            Some(&plan.id),
            &review,
        )?;
        run.reviews.push(ReviewRef {
            id: review.id.clone(),
            revision,
            score: review.score,
            pass: review.pass,
            findings: u32::try_from(review.findings.len()).unwrap_or(u32::MAX),
            cycle,
        });
        Self::set_checkpoint(
            run,
            "last_review",
            serde_json::to_value(&review).unwrap_or(Value::Null),
        );
        let mut history: Vec<Review> = Vec::new();
        for r in &run.reviews {
            if let Some(rv) =
                records.latest::<Review>(KIND_RUN_REVIEW, &format!("{}:{}", run.id, r.cycle))?
            {
                history.push(rv);
            }
        }
        let mut r = StageResult::ok(Outcome::ReviewPass);
        r.events.push(("review_ready".into(), json!({"review": review.id, "score": review.score, "pass": review.pass, "findings": review.findings.len()})));
        r.output = json!({"review": review.id, "score": review.score, "pass": review.pass});
        if review.pass {
            if run.policy.final_approval
                && !run
                    .approvals
                    .iter()
                    .any(|a| a.kind == DecisionKind::FinalApproval && a.option == "approve")
            {
                let d = pending(
                    run,
                    format!("dec-{}-final", run.id),
                    DecisionKind::FinalApproval,
                    "The review passed. Approve the final output?",
                    vec![
                        DecisionOption::new("approve", "Approve"),
                        DecisionOption::new("stop", "Stop (keep the result, do not complete)"),
                    ],
                    json!({"review": review, "sequences": run.sequences}),
                    "Approving completes the run; everything stays editable.",
                    None,
                    RunStage::Review,
                    None,
                );
                return Ok(StageResult::wait(d));
            }
            return Ok(r);
        }
        let loops_left = run.budget.review_loops_left(&run.usage);
        let osc = critic::oscillating(&history);
        if review.needs_replan() && run.budget.replans_left(&run.usage) {
            Self::replan_feedback(
                run,
                "review_requires_replan",
                &review
                    .open_blocking()
                    .filter(|f| f.needs_replan)
                    .map(|f| format!("{}: {}", f.key, f.observed))
                    .collect::<Vec<_>>(),
            );
            r.outcome = Outcome::ReviewReplan;
            return Ok(r);
        }
        if review.open_actionable().next().is_some() && loops_left && !osc {
            run.usage.review_loops += 1;
            r.outcome = Outcome::ReviewActionable;
            return Ok(r);
        }
        // sem melhora/oscilação/limite: pára e pede decisão (nunca laço infinito)
        let d = pending(
            run,
            format!("dec-{}-review-{}", run.id, cycle),
            DecisionKind::FinalApproval,
            if osc {
                "The corrections are not improving the result."
            } else {
                "The review loops are exhausted with findings still open."
            },
            vec![
                DecisionOption::new(
                    "accept_with_warnings",
                    "Accept the result with the open findings",
                ),
                DecisionOption::new("extend", "Allow one more correction loop"),
                DecisionOption::new("stop", "Stop the run"),
            ],
            json!({"review": review, "oscillating": osc, "loops": run.usage.review_loops}),
            "Accepting keeps the timeline as it is (fully editable); the findings stay listed in the report.",
            None,
            RunStage::Review,
            Some("accept_with_warnings"),
        );
        Ok(StageResult::wait(d))
    }

    // ---- CORRECT -------------------------------------------------------------------------------

    async fn st_correct(
        &self,
        run: &mut AiRun,
        ic: &IntelCtx,
        flags: &Flags,
    ) -> IntelResult<StageResult> {
        let _ = ic;
        let review: Review = serde_json::from_value(run.checkpoint["last_review"].clone())
            .map_err(|_| IntelError::new("PLAN_CONTEXT_MISSING", "no review to correct"))?;
        let findings: Vec<&critic::Finding> = review.open_actionable().collect();
        let cycle = run.usage.review_loops;
        let cp = critic::compile_correction(&run.id, cycle, &findings);
        if cp.commands.is_empty() {
            Self::replan_feedback(run, "no_automatic_fix", &cp.skipped);
            return Ok(StageResult::ok(Outcome::CorrectReplan));
        }
        let actor = Self::actor(run);
        self.ensure_running(&run.id)?;
        if flags.cancel.is_cancelled() {
            return Err(IntelError::cancelled());
        }
        let label = format!("AI Run {}: correction {cycle}", run.id);
        let pv = match self
            .deps
            .engine
            .preview(&actor, &label, Value::Array(cp.commands.clone()))
        {
            Ok(p) => p,
            Err(e) => {
                Self::replan_feedback(
                    run,
                    "correction_conflict",
                    &[format!("{}: {}", e.message, e.code)],
                );
                return Ok(StageResult::ok(Outcome::Conflict));
            }
        };
        if !pv["already_applied"].as_bool().unwrap_or(false) {
            let token = pv["plan_token"]
                .as_str()
                .ok_or_else(|| IntelError::new("PLAN_TOKEN_MISSING", "no token"))?
                .to_owned();
            fp!("autonomy_correct_before_apply");
            self.ensure_running(&run.id)?;
            let reply = match self.deps.engine.apply(&actor, &token) {
                Ok(r) => r,
                Err(e) => {
                    Self::replan_feedback(
                        run,
                        "correction_apply_failed",
                        &[format!("{}: {}", e.message, e.code)],
                    );
                    return Ok(StageResult::ok(Outcome::Conflict));
                }
            };
            let rev = reply["revision"].as_u64().unwrap_or(0);
            run.applied.push(AppliedRef {
                stage: RunStage::Correct,
                deliverable: format!("correction-{cycle}"),
                entry_id: self.last_entry_of(&run.actor_id()),
                revision_before: rev.saturating_sub(1),
                revision_after: rev,
                operation_namespace: format!("{}:c{cycle}", run.id),
                cycle,
            });
        }
        let mut r = StageResult::ok(Outcome::CorrectOk);
        r.events.push((
            "correction_applied".into(),
            json!({"cycle": cycle, "findings": cp.finding_refs, "affected": cp.affected_clips}),
        ));
        r.output = json!({"correction": cp.id, "findings": cp.finding_refs});
        Ok(r)
    }

    // ---- decisões ------------------------------------------------------------------------------

    /// Efeito de uma decisão humana sobre a Run (chamado dentro de `mutate`, com CAS).
    pub(super) fn apply_decision(
        &self,
        run: &mut AiRun,
        p: &PendingDecision,
        option: &str,
        payload: &Value,
        events: &mut Vec<(String, Value)>,
    ) -> IntelResult<()> {
        use super::machine::RunStatus;
        let go = |run: &mut AiRun, stage: RunStage| {
            run.status = RunStatus::Running;
            run.stage = stage;
            run.resume_stage = None;
            run.current_attempt += 1;
        };
        let cancel = |run: &mut AiRun, events: &mut Vec<(String, Value)>| {
            run.status = RunStatus::Cancelled;
            run.completed_ms = Some(now_ms());
            events.push(("run_cancelled".into(), json!({"by_decision": p.id})));
        };
        match p.kind {
            DecisionKind::OpenQuestion => {
                if option == "answer" {
                    let answers: Vec<String> = payload["answers"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(|s| s.chars().take(2_000).collect()))
                                .collect()
                        })
                        .or_else(|| {
                            payload["text"]
                                .as_str()
                                .map(|t| vec![t.chars().take(4_000).collect()])
                        })
                        .unwrap_or_default();
                    if answers.is_empty() {
                        return Err(IntelError::new(
                            "INVALID_ARGUMENT",
                            "provide `answers` (list) or `text`",
                        ));
                    }
                    // resposta a "sem briefing" vira o brief
                    if p.context["missing"] == "brief" {
                        run.inputs.brief_text = Some(answers.join("\n"));
                    } else {
                        run.inputs.answers.extend(answers);
                    }
                    run.question_rounds += 1;
                    Self::set_checkpoint(run, "reinterpret", json!(true));
                } else if let Some(v) = run.demand_spec_version {
                    Self::set_checkpoint(run, "spec_approved", json!(v));
                    // aprovar com perguntas abertas encerra as rodadas (segue com as premissas)
                    run.question_rounds = run.policy.max_question_rounds;
                }
                go(run, RunStage::Understand);
            }
            DecisionKind::PlanApproval => match option {
                "approve" => go(run, RunStage::ValidatePlan),
                "change" => {
                    Self::set_checkpoint(
                        run,
                        "replan",
                        json!({"reason": "user_requested_changes", "comment": payload["comment"].as_str().unwrap_or("").chars().take(1_000).collect::<String>()}),
                    );
                    go(run, RunStage::Plan);
                }
                _ => cancel(run, events),
            },
            DecisionKind::SpendApproval
            | DecisionKind::AssetApproval
            | DecisionKind::GenerationApproval => {
                if option == "approve" {
                    go(run, p.resume_stage);
                } else if matches!(p.kind, DecisionKind::SpendApproval)
                    && p.resume_stage == RunStage::ValidatePlan
                {
                    cancel(run, events);
                } else {
                    // candidato/geração recusados: não voltam a ser propostos
                    let key = p.context["candidate"]["adapter"]
                        .as_str()
                        .zip(p.context["candidate"]["id"].as_str())
                        .map(|(a, i)| format!("{a}:{i}"));
                    if !run.checkpoint["rejected"].is_array() {
                        Self::set_checkpoint(run, "rejected", json!([]));
                    }
                    if let (Some(k), Some(arr)) = (key, run.checkpoint["rejected"].as_array_mut()) {
                        arr.push(json!(k));
                    }
                    if p.kind == DecisionKind::GenerationApproval
                        && let Some(n) = p.context["need"].as_str()
                        && let Some(need) = run
                            .production_plan
                            .as_mut()
                            .and_then(|pl| pl.asset_needs.iter_mut().find(|x| x.id == n))
                    {
                        need.source_priority
                            .retain(|s| *s != AcquireSource::Generate);
                    }
                    go(run, p.resume_stage);
                }
            }
            DecisionKind::BudgetExtension => {
                if option == "extend" {
                    if let Some(c) = payload["max_cost_micros"].as_u64() {
                        run.budget.max_cost_micros = Some(c);
                    } else if let Some(c) = run.budget.max_cost_micros {
                        run.budget.max_cost_micros = Some(c.saturating_mul(2).max(c + 1));
                    }
                    run.budget.max_replans += 1;
                    run.budget.max_generations = run.budget.max_generations.map(|g| g + 1);
                    run.budget.max_tokens = run.budget.max_tokens.map(|t| t.saturating_mul(2));
                    run.budget.max_provider_calls =
                        run.budget.max_provider_calls.map(|c| c.saturating_mul(2));
                    run.budget.max_wall_time_ms =
                        run.budget.max_wall_time_ms.map(|t| t.saturating_mul(2));
                    go(run, p.resume_stage);
                } else {
                    cancel(run, events);
                }
            }
            DecisionKind::ConflictResolution => match option {
                "retry" => go(run, p.resume_stage),
                "replan" => {
                    Self::set_checkpoint(
                        run,
                        "replan",
                        json!({"reason": "user_chose_replan", "context": p.context}),
                    );
                    go(run, RunStage::Plan);
                }
                "accept" => {
                    // aceita o plano repetido: segue para validar
                    go(run, RunStage::ValidatePlan);
                }
                _ => cancel(run, events),
            },
            DecisionKind::FinalApproval => match option {
                "approve" | "accept_with_warnings" => {
                    run.status = RunStatus::Completed;
                    run.stage = RunStage::Done;
                    run.completed_ms = Some(now_ms());
                    run.report =
                        Some(self.report_with_warnings(run, option == "accept_with_warnings"));
                    events.push(("run_done".into(), json!({"report": run.report})));
                }
                "extend" => {
                    run.budget.max_review_loops += 1;
                    go(run, RunStage::Review);
                }
                _ => cancel(run, events),
            },
        }
        Ok(())
    }

    fn report_with_warnings(&self, run: &AiRun, warnings: bool) -> Value {
        let mut r = serde_json::to_value(self.summary(run)).unwrap_or(Value::Null);
        r["accepted_with_warnings"] = json!(warnings);
        r["transactions"] = json!(run.applied);
        r["reviews"] = json!(run.reviews);
        r
    }
}

fn gateway_sha(path: &std::path::Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read as _;
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    let mut s = String::from("sha256:");
    for b in h.finalize() {
        s.push_str(&format!("{b:02x}"));
    }
    Some(s)
}
