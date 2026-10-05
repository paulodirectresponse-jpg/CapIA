//! Superfície `ai.run.*` / `ai.memory.*` / `ai.gateway.*` / `ai.generation.*` (Fase 5). Mesmas
//! regras da Fase 4: a UI só fala com isto por `aiController`; nada devolve segredo; com a IA
//! desligada ou sem Run, o editor é idêntico e **nenhuma Run começa sozinha** (recuperação só
//! classifica: Runs interrompidas ficam `paused`, nunca retomam sem comando explícito).

use super::{IntelligenceService, bad, lock, parse};
use crate::autonomy::gateway::{ApprovedUrlAdapter, GatewayRegistry, LocalLibraryAdapter};
use crate::autonomy::generation::GenerationRegistry;
use crate::autonomy::machine::RunStatus;
use crate::autonomy::memory::{
    MemoryDraft, MemoryKind, MemoryScope, MemorySource, MemoryStatus, UserApproval,
};
use crate::autonomy::model::{AiRun, RunBudget, RunInputs, RunPolicy, VariantRequest};
use crate::autonomy::orchestrator::{Deps, Orchestrator, store_err};
use crate::error::{IntelError, IntelResult};
use capia_ai::fetch::FetchPolicy;
use capia_store::AppDb;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

const NS_GW: &str = "gateway";
const KEY_GW: &str = "config";
const MAX_CONCURRENT_RUNS: usize = 4;
const MAX_RUNS_PER_GROUP: usize = 100;

/// Hosts de uma fonte aprovada: nome DNS (ou `*.nome`), nunca IP/URL/credencial.
fn valid_host(h: &str) -> bool {
    let core = h.strip_prefix("*.").unwrap_or(h);
    !core.is_empty()
        && core.len() <= 253
        && core.contains('.')
        && core.parse::<std::net::IpAddr>().is_err()
        && core.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        && !core.ends_with(".local")
        && !core.ends_with(".internal")
        && !core.ends_with(".localdomain")
}

/// Carrega do `AppDb` as fontes configuradas (biblioteca local, URLs aprovadas) e os desligados.
pub(super) fn load_gateway_config(
    db: Option<&AppDb>,
    gw: &GatewayRegistry,
    gens: &GenerationRegistry,
) {
    // geração externa gasta dinheiro: desligada até o usuário ligar (e persistir) explicitamente
    gens.set_enabled(false);
    let Some(v) = db.and_then(|d| d.get(NS_GW, KEY_GW).ok().flatten()) else {
        return;
    };
    for lib in v["libraries"].as_array().into_iter().flatten() {
        if let (Some(id), Some(path)) = (lib["id"].as_str(), lib["path"].as_str()) {
            gw.register_shared(Arc::new(LocalLibraryAdapter::new(id, PathBuf::from(path))));
        }
    }
    for src in v["url_sources"].as_array().into_iter().flatten() {
        let hosts: Vec<String> = src["hosts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| h.as_str().filter(|h| valid_host(h)).map(str::to_owned))
            .collect();
        if let (Some(id), false) = (src["id"].as_str(), hosts.is_empty())
            && let Ok(a) = ApprovedUrlAdapter::new(
                id,
                FetchPolicy {
                    allowed_hosts: hosts,
                    ..FetchPolicy::default()
                },
            )
        {
            gw.register_shared(Arc::new(a));
        }
    }
    for d in v["disabled"].as_array().into_iter().flatten() {
        if let Some(id) = d.as_str() {
            gw.set_enabled(id, false);
        }
    }
    if v["generation_enabled"].as_bool() == Some(true) {
        gens.set_enabled(true);
    }
}

#[derive(Deserialize)]
struct CreateP {
    inputs: RunInputs,
    #[serde(default)]
    policy: Option<RunPolicy>,
    #[serde(default)]
    budget: Option<RunBudget>,
    #[serde(default = "yes")]
    start: bool,
}

fn yes() -> bool {
    true
}

fn run_id(p: &Value) -> IntelResult<&str> {
    p["run_id"]
        .as_str()
        .ok_or_else(|| bad("`run_id` is required"))
}

fn scope_of(p: &Value, k: &str) -> IntelResult<Option<MemoryScope>> {
    match p[k].as_str() {
        None => Ok(None),
        Some(s) => MemoryScope::parse(s)
            .map(Some)
            .ok_or_else(|| bad(format!("unknown memory scope `{s}`"))),
    }
}

impl IntelligenceService {
    fn ensure_ai_on(&self) -> IntelResult<()> {
        let reg = self.ai.registry();
        if !reg.ai_enabled {
            return Err(IntelError::new(
                "AI_DISABLED",
                "AI is turned off: the editor works fully without it",
            ));
        }
        if !reg.models.values().any(|m| reg.usable(m)) {
            return Err(IntelError::new(
                "NO_BRAIN",
                "no usable model is configured: set up a provider first",
            ));
        }
        Ok(())
    }

    /// Orchestrator do projeto aberto (reaproveitado por caminho; abrir = recuperar/classificar).
    pub(super) fn orch(&self) -> IntelResult<Arc<Orchestrator>> {
        let path = self
            .engine
            .project_path()
            .ok_or_else(|| IntelError::new("NO_PROJECT", "no project is open"))?;
        let mut g = lock(&self.autonomy);
        if let Some((p, o)) = g.as_ref()
            && *p == path
        {
            return Ok(o.clone());
        }
        let events = self.events.clone();
        let deps = Deps {
            engine: self.engine.clone(),
            ai: self.ai.clone(),
            gateways: self.gateways.clone(),
            generators: self.generators.clone(),
            app_db: self.appdb.clone(),
            sink: Arc::new(move |v| Self::emit(&events, v)),
            rt: self.rt.handle().clone(),
        };
        let o = Orchestrator::open(deps, path.clone())?;
        // abrir o projeto classifica as Runs interrompidas; NENHUMA retoma sozinha
        for r in o.recover()? {
            Self::emit(
                &self.events,
                json!({"kind": "ai_run", "run_id": r.run_id, "event": "run_recovered", "data": r}),
            );
        }
        *g = Some((path, o.clone()));
        Ok(o)
    }

    fn persist_gateway_config(&self, libs: Vec<Value>, urls: Vec<Value>) -> IntelResult<()> {
        if let Some(db) = &self.appdb {
            let disabled: Vec<String> = self
                .gateways
                .ids()
                .into_iter()
                .filter(|i| !self.gateways.is_enabled(i))
                .collect();
            db.put(
                NS_GW,
                KEY_GW,
                &json!({"libraries": libs, "url_sources": urls, "disabled": disabled,
                        "generation_enabled": self.generators.is_enabled()}),
                crate::records::now_ms(),
            )?;
        }
        Ok(())
    }

    fn gateway_config(&self) -> (Vec<Value>, Vec<Value>) {
        let v = self
            .appdb
            .as_ref()
            .and_then(|d| d.get(NS_GW, KEY_GW).ok().flatten())
            .unwrap_or(Value::Null);
        (
            v["libraries"].as_array().cloned().unwrap_or_default(),
            v["url_sources"].as_array().cloned().unwrap_or_default(),
        )
    }

    fn gateway_status(&self) -> Value {
        let adapters: Vec<Value> = self
            .gateways
            .ids()
            .into_iter()
            .filter_map(|id| {
                let a = self.gateways.get(&id)?;
                Some(
                    json!({"id": id, "kind": a.kind(), "enabled": self.gateways.is_enabled(&id),
                            "hosts": a.allowed_hosts(), "paid": a.is_paid()}),
                )
            })
            .collect();
        json!({"adapters": adapters,
               "generation": {"enabled": self.generators.is_enabled(), "available": self.generators.available()}})
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn autonomy_call(&self, method: &str, p: Value) -> IntelResult<Value> {
        match method {
            // ---- runs -------------------------------------------------------------------------
            "ai.run.create" => {
                self.ensure_ai_on()?;
                let q: CreateP = parse(p)?;
                if q.inputs.documents.len() > 12
                    || q.inputs.assets.len() > 50
                    || q.inputs.references.len() > 8
                {
                    return Err(bad("too many documents/assets/references"));
                }
                if q.inputs
                    .brief_text
                    .as_ref()
                    .is_some_and(|t| t.chars().count() > 60_000)
                {
                    return Err(bad("the brief is too long (60000 characters max)"));
                }
                if q.inputs.deliverables.len() > 24 {
                    return Err(bad("at most 24 deliverables per run"));
                }
                let o = self.orch()?;
                let running = o
                    .list(200)?
                    .iter()
                    .filter(|r| r["status"] == "running")
                    .count();
                if running >= MAX_CONCURRENT_RUNS {
                    return Err(IntelError::new(
                        "TOO_MANY_RUNS",
                        "too many runs are running; wait for one to finish",
                    ));
                }
                let profile = self
                    .ai
                    .registry()
                    .active_profile
                    .clone()
                    .unwrap_or_else(|| "none".into());
                let run = o.create_run(q.inputs, q.policy, q.budget, &profile, None)?;
                if q.start {
                    o.start(&run.id)?;
                }
                Ok(json!({"run": o.summary(&run)}))
            }
            "ai.run.start" => {
                self.ensure_ai_on()?;
                let o = self.orch()?;
                o.start(run_id(&p)?)?;
                Ok(json!({"run": o.summary(&o.load(run_id(&p)?)?)}))
            }
            "ai.run.list" => {
                let o = self.orch()?;
                let limit =
                    u32::try_from(p["limit"].as_u64().unwrap_or(100).min(500)).unwrap_or(100);
                Ok(json!({"runs": o.list(limit)?}))
            }
            "ai.run.get" => self.orch()?.snapshot(run_id(&p)?),
            "ai.run.events" => {
                let o = self.orch()?;
                Ok(
                    json!({"events": o.events_after(run_id(&p)?, p["after"].as_u64().unwrap_or(0))?}),
                )
            }
            "ai.run.pause" => {
                let o = self.orch()?;
                o.pause(run_id(&p)?)?;
                Ok(json!({"run": o.summary(&o.load(run_id(&p)?)?)}))
            }
            "ai.run.resume" => {
                self.ensure_ai_on()?;
                let o = self.orch()?;
                let r = o.resume(run_id(&p)?)?;
                Ok(json!({"run": o.summary(&r)}))
            }
            "ai.run.cancel" => {
                let o = self.orch()?;
                o.cancel(run_id(&p)?)?;
                Ok(json!({"run": o.summary(&o.load(run_id(&p)?)?)}))
            }
            "ai.run.decide" => {
                self.ensure_ai_on()?;
                let o = self.orch()?;
                let decision = p["decision_id"]
                    .as_str()
                    .ok_or_else(|| bad("`decision_id` is required"))?;
                let option = p["option"]
                    .as_str()
                    .ok_or_else(|| bad("`option` is required"))?;
                let r = o.decide(run_id(&p)?, decision, option, &p["payload"], "user")?;
                Ok(json!({"run": o.summary(&r)}))
            }
            "ai.run.review_decision" => {
                let o = self.orch()?;
                let key = p["finding_key"]
                    .as_str()
                    .ok_or_else(|| bad("`finding_key` is required"))?;
                let d = p["decision"]
                    .as_str()
                    .ok_or_else(|| bad("`decision` is required"))?;
                Ok(json!({"run": o.summary(&o.review_decision(run_id(&p)?, key, d)?)}))
            }
            "ai.run.rerun" => {
                self.ensure_ai_on()?;
                let o = self.orch()?;
                let budget: Option<RunBudget> = p
                    .get("budget")
                    .filter(|v| !v.is_null())
                    .map(|v| parse(v.clone()))
                    .transpose()?;
                let run = o.duplicate(
                    run_id(&p)?,
                    p["brief"].as_str().map(str::to_owned),
                    budget,
                    None,
                )?;
                if p["start"].as_bool().unwrap_or(true) {
                    o.start(&run.id)?;
                }
                Ok(json!({"run": o.summary(&run)}))
            }
            "ai.run.variants" => {
                self.ensure_ai_on()?;
                let o = self.orch()?;
                let parent = o.load(run_id(&p)?)?;
                if parent.status != RunStatus::Completed {
                    return Err(IntelError::new(
                        "INVALID_STATE",
                        "variants come from a completed run",
                    ));
                }
                let count = p["count"].as_u64().unwrap_or(0);
                if count == 0 || count > 20 {
                    return Err(bad("`count` must be between 1 and 20"));
                }
                let axis: Vec<String> = p["axis"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let group = parent
                    .variant_group_id
                    .clone()
                    .unwrap_or_else(|| format!("vg-{}", parent.id));
                if o.store().runs_in_group(&group).map_err(store_err)?.len() >= MAX_RUNS_PER_GROUP {
                    return Err(IntelError::new(
                        "TOO_MANY_RUNS",
                        "this variant group is full",
                    ));
                }
                let mut inputs = parent.inputs.clone();
                inputs.variants = Some(VariantRequest {
                    count: u32::try_from(count).unwrap_or(1),
                    axis,
                });
                inputs.demand_spec_id = parent.demand_spec_id.clone();
                inputs.master_sequence = parent.sequences.first().map(|s| s.sequence_id.clone());
                let run = o.create_run(
                    inputs,
                    Some(parent.policy.clone()),
                    Some(parent.budget.clone()),
                    &parent.brain_profile_id,
                    Some((&parent.id, Some(&group))),
                )?;
                o.start(&run.id)?;
                Ok(json!({"run": o.summary(&run), "variant_group": group}))
            }
            "ai.run.group" => {
                let o = self.orch()?;
                let g = p["variant_group"]
                    .as_str()
                    .ok_or_else(|| bad("`variant_group` is required"))?;
                let rows = o.store().runs_in_group(g).map_err(store_err)?;
                Ok(json!({"runs": rows
                    .iter()
                    .filter_map(|r| serde_json::from_value::<AiRun>(r.json.clone()).ok())
                    .map(|r| o.summary(&r))
                    .collect::<Vec<_>>()}))
            }
            "ai.run.cleanup" => {
                let o = self.orch()?;
                Ok(json!({"assets": o.cleanup_candidates(run_id(&p)?)?}))
            }
            "ai.run.provenance" => {
                let o = self.orch()?;
                let rows = match p["asset_id"].as_str() {
                    Some(a) => o
                        .store()
                        .get_provenance(a)
                        .map_err(store_err)?
                        .into_iter()
                        .collect(),
                    None => o
                        .store()
                        .list_provenance(p["run_id"].as_str())
                        .map_err(store_err)?,
                };
                Ok(json!({"provenance": rows}))
            }
            // ---- memória ----------------------------------------------------------------------
            "ai.memory.list" => {
                let o = self.orch()?;
                let status = match p["status"].as_str() {
                    None => None,
                    Some("proposed") => Some(MemoryStatus::Proposed),
                    Some("active") => Some(MemoryStatus::Active),
                    Some("rejected") => Some(MemoryStatus::Rejected),
                    Some("archived") => Some(MemoryStatus::Archived),
                    Some(s) => return Err(bad(format!("unknown status `{s}`"))),
                };
                Ok(json!({"items": o.memory().list(scope_of(&p, "scope")?, status)?}))
            }
            "ai.memory.add" => {
                // ação EXPLÍCITA do usuário (formulário "nova memória"): propõe e aprova no mesmo gesto
                let o = self.orch()?;
                let scope = scope_of(&p, "scope")?.ok_or_else(|| bad("`scope` is required"))?;
                let content = p["content"]
                    .as_str()
                    .ok_or_else(|| bad("`content` is required"))?;
                let item = o.memory().propose(
                    MemoryDraft {
                        scope,
                        client_id: p["client_id"].as_str().map(str::to_owned),
                        kind: MemoryKind::parse(p["kind"].as_str().unwrap_or("preference")),
                        content: content.to_owned(),
                        structured: None,
                        source: MemorySource::UserDeclared,
                        confidence: 1.0,
                        evidence: vec![],
                        key: p["key"].as_str().map(str::to_owned),
                    },
                    None,
                    false,
                )?;
                let item = o.memory().approve(
                    &item.id,
                    None,
                    None,
                    None,
                    &UserApproval::explicit_from_ui("user"),
                )?;
                Ok(json!({"item": item}))
            }
            "ai.memory.approve" => {
                let o = self.orch()?;
                let id = p["id"].as_str().ok_or_else(|| bad("`id` is required"))?;
                let item = o.memory().approve(
                    id,
                    scope_of(&p, "scope")?,
                    p["client_id"].as_str(),
                    p["content"].as_str(),
                    &UserApproval::explicit_from_ui("user"),
                )?;
                Ok(json!({"item": item}))
            }
            "ai.memory.reject" | "ai.memory.archive" | "ai.memory.delete" | "ai.memory.edit" => {
                let o = self.orch()?;
                let id = p["id"].as_str().ok_or_else(|| bad("`id` is required"))?;
                let a = UserApproval::explicit_from_ui("user");
                match method {
                    "ai.memory.reject" => Ok(json!({"item": o.memory().reject(id, &a)?})),
                    "ai.memory.archive" => Ok(json!({"item": o.memory().archive(id, &a)?})),
                    "ai.memory.delete" => Ok(json!({"deleted": o.memory().delete(id, &a)?})),
                    _ => {
                        let c = p["content"]
                            .as_str()
                            .ok_or_else(|| bad("`content` is required"))?;
                        Ok(json!({"item": o.memory().edit(id, c, &a)?}))
                    }
                }
            }
            "ai.memory.audit" => {
                let o = self.orch()?;
                Ok(json!({"log": o.memory().audit(p["id"].as_str())?}))
            }
            // ---- gateway / geração ------------------------------------------------------------
            "ai.gateway.status" => Ok(self.gateway_status()),
            "ai.gateway.set_enabled" => {
                let id = p["id"].as_str().ok_or_else(|| bad("`id` is required"))?;
                if self.gateways.get(id).is_none() {
                    return Err(IntelError::new("NOT_FOUND", "unknown gateway adapter"));
                }
                self.gateways.set_enabled(
                    id,
                    p["enabled"]
                        .as_bool()
                        .ok_or_else(|| bad("`enabled` is required"))?,
                );
                let (l, u) = self.gateway_config();
                self.persist_gateway_config(l, u)?;
                Ok(self.gateway_status())
            }
            "ai.gateway.add_library" => {
                let id = p["id"].as_str().ok_or_else(|| bad("`id` is required"))?;
                let path = PathBuf::from(
                    p["path"]
                        .as_str()
                        .ok_or_else(|| bad("`path` is required"))?,
                );
                if !path.is_dir() {
                    return Err(bad("the library path must be an existing folder"));
                }
                self.gateways
                    .register_shared(Arc::new(LocalLibraryAdapter::new(id, path.clone())));
                let (mut l, u) = self.gateway_config();
                l.retain(|x| x["id"] != id);
                l.push(json!({"id": id, "path": path.display().to_string()}));
                self.persist_gateway_config(l, u)?;
                Ok(self.gateway_status())
            }
            "ai.gateway.add_url_source" => {
                let id = p["id"].as_str().ok_or_else(|| bad("`id` is required"))?;
                let hosts: Vec<String> = p["hosts"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|h| h.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                if hosts.is_empty() || hosts.iter().any(|h| !valid_host(h)) {
                    return Err(bad(
                        "hosts must be DNS names (or `*.name`), never IPs, URLs or internal names",
                    ));
                }
                let a = ApprovedUrlAdapter::new(
                    id,
                    FetchPolicy {
                        allowed_hosts: hosts.clone(),
                        ..FetchPolicy::default()
                    },
                )
                .map_err(|e| bad(e.message))?;
                self.gateways.register_shared(Arc::new(a));
                let (l, mut u) = self.gateway_config();
                u.retain(|x| x["id"] != id);
                u.push(json!({"id": id, "hosts": hosts}));
                self.persist_gateway_config(l, u)?;
                Ok(self.gateway_status())
            }
            "ai.gateway.remove" => {
                let id = p["id"].as_str().ok_or_else(|| bad("`id` is required"))?;
                self.gateways.remove(id);
                let (mut l, mut u) = self.gateway_config();
                l.retain(|x| x["id"] != id);
                u.retain(|x| x["id"] != id);
                self.persist_gateway_config(l, u)?;
                Ok(self.gateway_status())
            }
            "ai.generation.set_enabled" => {
                self.generators.set_enabled(
                    p["enabled"]
                        .as_bool()
                        .ok_or_else(|| bad("`enabled` is required"))?,
                );
                let (l, u) = self.gateway_config();
                self.persist_gateway_config(l, u)?;
                Ok(self.gateway_status())
            }
            other => Err(IntelError::new(
                "UNKNOWN_METHOD",
                format!("unknown method `{other}`"),
            )),
        }
    }
}
