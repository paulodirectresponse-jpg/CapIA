//! Papéis de LLM da autonomia: **Producer**, **Planner** e o lado **semântico** do Critic
//! (PHASE5_PLANNER_EDITOR_CRITIC §1..§13, §44..§50). O Editor não é LLM: compila `EditPlan` de forma
//! determinística (`plan.rs`).
//!
//! Cada papel tem prompt **versionado**, saída com JSON Schema, **zero tools** (não há como um
//! texto hostil acionar nada) e contexto limitado; entradas externas (DemandSpec, transcrições,
//! nomes de arquivo, metadados de fontes) entram como `untrusted_data`. Nenhum papel escreve.

use super::critic::{
    Category, Evidence, Finding, FindingSource, FindingStatus, FixAction, RangeTicks, Severity,
};
use super::plan::{
    EditPlan, ProductionPlan, edit_plan_from, edit_plan_schema, production_plan_from,
    production_plan_schema,
};
use crate::ctx::IntelCtx;
use crate::error::{IntelError, IntelResult};
use capia_ai::brain::DataClass;
use capia_ai::capability::Capability;
use capia_ai::dispatcher::{ChatOptions, TaskCtx};
use capia_ai::prompt::{Priority, Section, UNTRUSTED_PREAMBLE, build_context};
use capia_ai::router::RouteRequest;
use capia_ai::types::{ChatRequest, Message};
use serde_json::{Value, json};

pub const PRODUCER_PROMPT_VERSION: u32 = 1;
pub const PLANNER_PROMPT_VERSION: u32 = 1;
pub const CRITIC_PROMPT_VERSION: u32 = 1;
/// Orçamento de tokens de contexto por papel (nunca despeja o projeto inteiro).
pub const CONTEXT_BUDGET_TOKENS: usize = 24_000;

pub const PRODUCER_SYSTEM: &str = "ROLE: producer (prompt v1)\n\
You are the Producer of a video editing run. From the demand, the reference grammar, the asset inventory and the memory, \
decide the production strategy: which deliverables to make (standalone, hook_plus_master, shared_master, format_variant), \
which assets are still needed (prefer existing project assets; do not generate media unnecessarily) and the assumptions and risks. \
You only PLAN: you cannot edit, call tools or access files. Reply with ONE JSON document that matches the schema.";

pub const PLANNER_SYSTEM: &str = "ROLE: planner (prompt v1)\n\
You are the Planner of a video editing run. Turn the production strategy into an EditPlan for ONE deliverable: ordered beats \
(hook, body, proof, offer, cta...) with integer millisecond durations, the asset and source range of each beat (or an asset need \
that is not yet acquired), overlays, captions, optional transition (prefer hard cuts), framing and the music. Respect the format, \
must_include, must_avoid, the required CTA and the maximum duration. Only use asset ids that exist in the inventory. \
You only PLAN: you cannot edit, call tools or access files. Reply with ONE JSON document that matches the schema.";

pub const CRITIC_SYSTEM: &str = "ROLE: critic (prompt v1)\n\
You are the Critic of a video editing run. Compare the resulting timeline digest with the demand and the plan. Report only \
concrete findings with evidence (clip, beat, range, transcript, constraint). Categories: timing, pacing, sync, captions, framing, \
continuity, brand, brief, reference, audio, transition, asset_quality, cta, technical. Severity: info, minor, major, blocker. \
A blocker/major finding MUST have evidence. Optionally propose a fix from this closed list only: \
trim_to_duration, add_cta_text, delete_clip, set_property, set_clip_enabled. You cannot edit anything. \
Reply with ONE JSON document that matches the schema.";

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RoleMeta {
    pub endpoint_id: String,
    pub model_id: String,
    pub cost_micros: Option<u64>,
    pub tokens: u64,
    pub cache_hit: bool,
    pub prompt_version: u32,
    pub input_digest: String,
}

/// Cache durável de saídas de papéis (livro de efeitos da Run): depois de um crash, a chamada de
/// LLM já concluída **não** é refeita (nem paga de novo).
pub trait EffectCache: Send + Sync {
    fn load(&self, key: &str) -> Option<Value>;
    fn save(&self, key: &str, value: &Value);
}

#[derive(Clone, Debug)]
pub struct RoleOut<T> {
    pub value: T,
    pub meta: RoleMeta,
    /// Saída crua validada (para o livro de efeitos).
    pub raw: Value,
}

fn sections_to_prompt(sections: &[Section]) -> (String, String) {
    let (text, report) = build_context(CONTEXT_BUDGET_TOKENS, sections);
    let digest = super::plan::digest_value(&json!({"text": text, "included": report.included}));
    (text, digest)
}

fn route(data: &[DataClass]) -> ChatOptions {
    let mut r = RouteRequest::for_capability(Capability::TextGeneration);
    r.also_needs.push(Capability::StructuredOutput);
    r.data.extend(data.iter().copied());
    r.min_context = Some(u32::try_from(CONTEXT_BUDGET_TOKENS + 4_000).unwrap_or(u32::MAX));
    ChatOptions {
        route: r,
        cacheable: true,
    }
}

#[allow(clippy::too_many_arguments)]
async fn call(
    ctx: &IntelCtx,
    task: &TaskCtx,
    system: &str,
    sections: &[Section],
    schema_name: &str,
    schema: &Value,
    version: u32,
    cache: Option<(&dyn EffectCache, &str)>,
) -> IntelResult<(Value, RoleMeta)> {
    if let Some((c, key)) = cache
        && let Some(stored) = c.load(key)
        && let (Some(raw), Ok(meta)) = (
            stored.get("raw"),
            serde_json::from_value::<RoleMeta>(stored["meta"].clone()),
        )
    {
        return Ok((
            raw.clone(),
            RoleMeta {
                cache_hit: true,
                cost_micros: Some(0),
                tokens: 0,
                ..meta
            },
        ));
    }
    let (user, digest) = sections_to_prompt(sections);
    let mut req = ChatRequest::new(
        String::new(),
        vec![
            Message::system(format!("{system}\n\n{UNTRUSTED_PREAMBLE}")),
            Message::user(user),
        ],
    );
    req.params.temperature = Some(0.0);
    let (raw, out) = ctx
        .ai
        .chat_structured(
            task,
            req,
            schema_name,
            schema,
            route(&[DataClass::DocumentText]),
            1,
            None,
        )
        .await?;
    let meta = RoleMeta {
        endpoint_id: out.decision.endpoint_id.clone(),
        model_id: out.decision.model_id.clone(),
        cost_micros: out.cost.known.then_some(out.cost.micros),
        tokens: out.response.usage.input_tokens + out.response.usage.output_tokens,
        cache_hit: out.cache_hit,
        prompt_version: version,
        input_digest: digest,
    };
    if let Some((c, key)) = cache {
        c.save(key, &json!({"raw": raw, "meta": meta}));
    }
    Ok((raw, meta))
}

fn sec(priority: Priority, label: &str, text: String, trusted: bool) -> Section {
    Section {
        priority,
        label: label.to_owned(),
        text,
        trusted,
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// Contexto comum (tudo que veio de fora entra como `untrusted_data`).
#[derive(Debug)]
pub struct PlanningContext {
    pub demand: Value,
    pub reference: Option<Value>,
    pub inventory: Value,
    pub transcripts: Vec<(String, String)>,
    pub memory: Vec<String>,
    pub capabilities: Value,
    pub requested: Value,
    /// Replanejamento: motivo + plano anterior + erros de validação.
    pub feedback: Option<Value>,
}

fn common_sections(c: &PlanningContext) -> Vec<Section> {
    let mut v = vec![
        sec(
            Priority::UserRequest,
            "requested deliverables and budget",
            pretty(&c.requested),
            true,
        ),
        sec(
            Priority::Project,
            "capabilities available",
            pretty(&c.capabilities),
            true,
        ),
        sec(Priority::Project, "demand spec", pretty(&c.demand), false),
        sec(
            Priority::SelectedAssets,
            "asset inventory",
            pretty(&c.inventory),
            false,
        ),
    ];
    if let Some(r) = &c.reference {
        v.push(sec(
            Priority::Summaries,
            "reference grammar",
            pretty(r),
            false,
        ));
    }
    for (name, t) in &c.transcripts {
        v.push(sec(
            Priority::Summaries,
            &format!("transcript {name}"),
            t.clone(),
            false,
        ));
    }
    if !c.memory.is_empty() {
        v.push(sec(
            Priority::Project,
            "memory (system > user > client > project precedence reversed: project wins)",
            c.memory.join("\n"),
            false,
        ));
    }
    if let Some(f) = &c.feedback {
        v.push(sec(
            Priority::UserRequest,
            "replan feedback",
            pretty(f),
            true,
        ));
    }
    v
}

pub async fn run_producer(
    ctx: &IntelCtx,
    task: &TaskCtx,
    pc: &PlanningContext,
    plan_id: String,
    version: u32,
    demand_spec_version: Option<u32>,
    cache: Option<(&dyn EffectCache, &str)>,
) -> IntelResult<RoleOut<ProductionPlan>> {
    let secs = common_sections(pc);
    let (raw, meta) = call(
        ctx,
        task,
        PRODUCER_SYSTEM,
        &secs,
        "production_plan",
        &production_plan_schema(),
        PRODUCER_PROMPT_VERSION,
        cache,
    )
    .await?;
    let plan = production_plan_from(&raw, plan_id, version, demand_spec_version)
        .map_err(|e| IntelError::new(e.code, e.message))?;
    Ok(RoleOut {
        value: plan,
        meta,
        raw,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn run_planner(
    ctx: &IntelCtx,
    task: &TaskCtx,
    pc: &PlanningContext,
    production: &ProductionPlan,
    deliverable_key: &str,
    plan_id: String,
    version: u32,
    cache: Option<(&dyn EffectCache, &str)>,
) -> IntelResult<RoleOut<EditPlan>> {
    let mut secs = common_sections(pc);
    secs.insert(
        0,
        sec(
            Priority::UserRequest,
            "production plan (trusted: produced by the Producer role)",
            pretty(&serde_json::to_value(production).unwrap_or(Value::Null)),
            true,
        ),
    );
    secs.insert(
        1,
        sec(
            Priority::UserRequest,
            "target deliverable",
            json!({"deliverable_key": deliverable_key}).to_string(),
            true,
        ),
    );
    let (raw, meta) = call(
        ctx,
        task,
        PLANNER_SYSTEM,
        &secs,
        "edit_plan",
        &edit_plan_schema(),
        PLANNER_PROMPT_VERSION,
        cache,
    )
    .await?;
    let plan = edit_plan_from(&raw, plan_id, version, deliverable_key)
        .map_err(|e| IntelError::new(e.code, e.message))?;
    Ok(RoleOut {
        value: plan,
        meta,
        raw,
    })
}

pub fn critic_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "summary": {"type": "string"},
            "findings": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "key": {"type": "string"},
                    "severity": {"type": "string", "enum": ["info", "minor", "major", "blocker"]},
                    "category": {"type": "string"},
                    "expected": {"type": "string"},
                    "observed": {"type": "string"},
                    "confidence": {"type": "number"},
                    "needs_replan": {"type": "boolean"},
                    "range_start_ticks": {"type": ["integer", "null"]},
                    "range_end_ticks": {"type": ["integer", "null"]},
                    "evidence": {"type": "array", "items": {"type": "object", "properties": {
                        "kind": {"type": "string"}, "detail": {"type": "string"}}, "required": ["kind", "detail"]}},
                    "fix": {"type": ["object", "null"], "properties": {
                        "action": {"type": "string", "enum": ["trim_to_duration", "add_cta_text", "delete_clip", "set_property", "set_clip_enabled"]},
                        "clip": {"type": ["string", "null"]},
                        "sequence": {"type": ["string", "null"]},
                        "new_end_ticks": {"type": ["integer", "null"]},
                        "text": {"type": ["string", "null"]},
                        "start_ticks": {"type": ["integer", "null"]},
                        "duration_ticks": {"type": ["integer", "null"]},
                        "prop": {"type": ["string", "null"]},
                        "value": {"type": ["number", "null"]},
                        "enabled": {"type": ["boolean", "null"]}
                    }}
                },
                "required": ["key", "severity", "category", "expected", "observed"]
            }}
        },
        "required": ["findings"]
    })
}

fn category_of(s: &str) -> Category {
    match s {
        "timing" => Category::Timing,
        "pacing" => Category::Pacing,
        "sync" => Category::Sync,
        "captions" => Category::Captions,
        "framing" => Category::Framing,
        "continuity" => Category::Continuity,
        "brand" => Category::Brand,
        "reference" => Category::Reference,
        "audio" => Category::Audio,
        "transition" => Category::Transition,
        "asset_quality" => Category::AssetQuality,
        "cta" => Category::Cta,
        "technical" => Category::Technical,
        _ => Category::Brief,
    }
}

fn fix_from(v: &Value) -> Option<FixAction> {
    let s = |k: &str| v[k].as_str().map(str::to_owned);
    match v["action"].as_str()? {
        "trim_to_duration" => Some(FixAction::TrimToDuration {
            clip: s("clip")?,
            new_end_ticks: v["new_end_ticks"].as_i64()?,
        }),
        "add_cta_text" => Some(FixAction::AddCtaText {
            sequence: s("sequence")?,
            text: s("text")?,
            start_ticks: v["start_ticks"].as_i64()?,
            duration_ticks: v["duration_ticks"].as_i64()?,
        }),
        "delete_clip" => Some(FixAction::DeleteClip { clip: s("clip")? }),
        "set_property" => Some(FixAction::SetProperty {
            clip: s("clip")?,
            prop: s("prop")?,
            value: v["value"].as_f64()?,
        }),
        "set_clip_enabled" => Some(FixAction::SetClipEnabled {
            clip: s("clip")?,
            enabled: v["enabled"].as_bool()?,
        }),
        _ => None,
    }
}

/// Converte a saída (validada por schema) em achados — **com rebaixamento**: blocker/major sem
/// evidência concreta vira `minor` (nada de "não gostei" bloqueando o loop).
pub fn findings_from(raw: &Value, deliverable: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (i, f) in raw["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .take(40)
        .enumerate()
    {
        let key = f["key"]
            .as_str()
            .map_or_else(|| format!("sem{i}"), |k| k.chars().take(80).collect());
        let evidence: Vec<Evidence> = f["evidence"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| {
                Some(Evidence {
                    kind: e["kind"].as_str()?.to_owned(),
                    detail: json!(e["detail"].as_str()?),
                })
            })
            .collect();
        let mut sev = match f["severity"].as_str() {
            Some("blocker") => Severity::Blocker,
            Some("major") => Severity::Major,
            Some("minor") => Severity::Minor,
            _ => Severity::Info,
        };
        if sev.blocks() && evidence.is_empty() {
            sev = Severity::Minor;
        }
        let at = match (
            f["range_start_ticks"].as_i64(),
            f["range_end_ticks"].as_i64(),
        ) {
            (Some(a), Some(b)) if b >= a => Some(RangeTicks { start: a, end: b }),
            _ => None,
        };
        let fix = f.get("fix").filter(|x| x.is_object()).and_then(fix_from);
        out.push(Finding {
            id: format!("sem:{deliverable}:{key}"),
            key: format!("sem:{deliverable}:{key}"),
            severity: sev,
            category: category_of(f["category"].as_str().unwrap_or("brief")),
            source: FindingSource::Semantic,
            at,
            evidence,
            expected: f["expected"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(400)
                .collect(),
            observed: f["observed"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(400)
                .collect(),
            suggested_fix: fix,
            confidence: f["confidence"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0),
            status: FindingStatus::Open,
            needs_replan: f["needs_replan"].as_bool().unwrap_or(false),
            deliverable: Some(deliverable.to_owned()),
        });
    }
    out
}

#[derive(Debug)]
pub struct CriticContext {
    pub demand: Value,
    pub plan: Value,
    pub timeline_digest: Value,
    pub transcript: Option<String>,
    pub deterministic: Value,
    pub decisions: Value,
}

pub async fn run_semantic_critic(
    ctx: &IntelCtx,
    task: &TaskCtx,
    cc: &CriticContext,
    deliverable: &str,
    cache: Option<(&dyn EffectCache, &str)>,
) -> IntelResult<RoleOut<Vec<Finding>>> {
    let mut secs = vec![
        sec(
            Priority::UserRequest,
            "deterministic findings already known (do not repeat)",
            pretty(&cc.deterministic),
            true,
        ),
        sec(
            Priority::UserRequest,
            "user decisions (ignored/locked finding keys: do not insist)",
            pretty(&cc.decisions),
            true,
        ),
        sec(Priority::Project, "edit plan", pretty(&cc.plan), true),
        sec(Priority::Project, "demand spec", pretty(&cc.demand), false),
        sec(
            Priority::SelectedAssets,
            "timeline digest",
            pretty(&cc.timeline_digest),
            false,
        ),
    ];
    if let Some(t) = &cc.transcript {
        secs.push(sec(Priority::Summaries, "transcript", t.clone(), false));
    }
    let (raw, meta) = call(
        ctx,
        task,
        CRITIC_SYSTEM,
        &secs,
        "critic_review",
        &critic_schema(),
        CRITIC_PROMPT_VERSION,
        cache,
    )
    .await?;
    Ok(RoleOut {
        value: findings_from(&raw, deliverable),
        meta,
        raw,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn blocking_findings_without_evidence_are_downgraded() {
        let raw = json!({"findings": [
            {"key": "a", "severity": "blocker", "category": "pacing", "expected": "e", "observed": "o", "evidence": []},
            {"key": "b", "severity": "major", "category": "brief", "expected": "e", "observed": "o",
             "evidence": [{"kind": "beat", "detail": "hook"}], "fix": {"action": "delete_clip", "clip": "c1"}},
            {"key": "c", "severity": "major", "category": "x", "expected": "e", "observed": "o",
             "evidence": [{"kind": "clip", "detail": "c"}], "fix": {"action": "format_disk"}}
        ]});
        let f = findings_from(&raw, "main");
        assert_eq!(f[0].severity, Severity::Minor, "no evidence ⇒ cannot block");
        assert_eq!(f[1].severity, Severity::Major);
        assert!(matches!(
            f[1].suggested_fix,
            Some(FixAction::DeleteClip { .. })
        ));
        assert!(
            f[2].suggested_fix.is_none(),
            "unknown fix actions are dropped (closed vocabulary)"
        );
        assert!(
            f.iter()
                .all(|x| x.source == FindingSource::Semantic && x.key.starts_with("sem:main:"))
        );
    }

    #[test]
    fn schemas_are_valid_json_schema_documents() {
        for s in [
            production_plan_schema(),
            edit_plan_schema(),
            critic_schema(),
        ] {
            assert_eq!(s["type"], "object");
            assert!(s["required"].is_array());
        }
    }

    #[test]
    fn role_prompts_declare_the_role_and_forbid_acting() {
        for (p, role) in [
            (PRODUCER_SYSTEM, "producer"),
            (PLANNER_SYSTEM, "planner"),
            (CRITIC_SYSTEM, "critic"),
        ] {
            assert!(p.starts_with(&format!("ROLE: {role}")));
            assert!(p.contains("cannot edit"));
        }
    }
}
