use crate::PortError;
use crate::Transport;
use crate::translate::{AssetFacts, TranslateCtx, translate};
use crate::units::{LEGACY_TICKS_PER_SECOND, seconds_to_editor_ticks, to_legacy_ticks};
use capia_actor::{Actor, ActorKind};
use capia_intelligence::engine::{Engine, READ_METHODS};
use capia_intelligence::{IntelError, IntelResult};
use capia_media::MediaToolchain;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

struct Pending {
    plan: Value,
    actor: Actor,
}

/// `Engine` da inteligência sobre a AgentSurface do editor (um único documento: o da UI).
pub struct AgentSurfaceEngine<T: Transport> {
    transport: T,
    pending: Mutex<HashMap<String, Pending>>,
    applied_plans: Mutex<HashSet<String>>,
}

impl<T: Transport> core::fmt::Debug for AgentSurfaceEngine<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("AgentSurfaceEngine")
    }
}

fn sha_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            use core::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn actor_kind(a: &Actor) -> &'static str {
    match a.kind {
        ActorKind::Agent => "agent",
        ActorKind::Api => "api",
        ActorKind::User => "human",
        ActorKind::System => "cli",
    }
}

fn intel(e: PortError) -> IntelError {
    IntelError::new(e.code, e.message)
}

/// Resposta de superfície `{ok:false,error}` → erro estruturado.
fn surface_result(v: Value) -> IntelResult<Value> {
    if v["ok"].as_bool() == Some(false) {
        let e = &v["error"];
        return Err(IntelError::new(
            e["code"].as_str().unwrap_or("ENGINE_ERROR").to_owned(),
            e["message"].as_str().unwrap_or("engine error").to_owned(),
        ));
    }
    Ok(v)
}

impl<T: Transport> AgentSurfaceEngine<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            pending: Mutex::new(HashMap::new()),
            applied_plans: Mutex::new(HashSet::new()),
        }
    }

    fn call(&self, method: &str, params: Value) -> IntelResult<Value> {
        self.transport.call(method, params).map_err(intel)
    }

    fn project(&self) -> IntelResult<Value> {
        surface_result(self.call("inspect_project", json!({}))?)
    }

    fn translate_ctx(&self, project: &Value) -> IntelResult<TranslateCtx> {
        let mut ctx = TranslateCtx::default();
        for a in project["project"]["assets"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(id) = a["id"].as_str() {
                ctx.assets.insert(
                    id.to_owned(),
                    AssetFacts {
                        duration_editor_ticks: a["durationSeconds"]
                            .as_f64()
                            .map(seconds_to_editor_ticks),
                    },
                );
            }
        }
        let tracks = surface_result(self.call("inspect_tracks", json!({}))?)?;
        ctx.main_track_id = tracks["tracks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|t| t["zone"] == "main")
            .and_then(|t| t["id"].as_str().map(str::to_owned));
        Ok(ctx)
    }
}

impl<T: Transport> Engine for AgentSurfaceEngine<T> {
    fn read(&self, method: &str, _params: Value) -> IntelResult<Value> {
        if !READ_METHODS.contains(&method) {
            return Err(IntelError::new(
                "NOT_ALLOWED",
                format!("`{method}` is not a read method available to the AI"),
            ));
        }
        match method {
            "engine.info" => Ok(json!({
                "engine": "opencut-agent-surface", "surface_schema": 1,
                "ticks_per_second": LEGACY_TICKS_PER_SECOND,
            })),
            "project.snapshot" => {
                let p = self.project()?;
                Ok(json!({"revision": p["project"]["revision"], "project": p["project"]}))
            }
            "sequence.get" => surface_result(self.call("inspect_tracks", json!({}))?),
            "assets.list" => {
                let p = self.project()?;
                let rows: Vec<Value> = p["project"]["assets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|a| {
                        let kind = a["type"].as_str().unwrap_or("video");
                        json!({
                            "id": a["id"], "name": a["name"], "kind": kind, "status": "online",
                            "duration": a["durationSeconds"].as_f64()
                                .map(|s| to_legacy_ticks(seconds_to_editor_ticks(s))),
                            "has_video": kind == "video" || kind == "image",
                            "has_audio": kind == "audio" || kind == "video",
                        })
                    })
                    .collect();
                Ok(Value::Array(rows))
            }
            "history.list" => {
                let p = self.project()?;
                Ok(p["project"]["history"]["entries"].clone())
            }
            _ => Err(IntelError::new("NOT_ALLOWED", "unknown read method")),
        }
    }

    fn preview(&self, actor: &Actor, label: &str, commands: Value) -> IntelResult<Value> {
        let list = commands
            .as_array()
            .ok_or_else(|| IntelError::new("INVALID_COMMAND", "commands must be an array"))?;
        let project = self.project()?;
        let ctx = self.translate_ctx(&project)?;
        let translated = translate(list, &ctx).map_err(intel)?;
        let revision = project["project"]["revision"].as_i64().unwrap_or(0);
        let canonical = serde_json::to_string(list).unwrap_or_default();
        let plan_id = format!(
            "p-{}",
            &sha_hex(format!("{label}\n{canonical}").as_bytes())[..24]
        );
        let plan = json!({
            "schemaVersion": 1, "planId": plan_id,
            "actor": {"kind": actor_kind(actor), "id": actor.id},
            "baseRevision": revision, "operations": translated.operations,
        });
        let already = self
            .applied_plans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&plan_id);
        let report = self.call("preview_plan", json!({"plan": plan}))?;
        if report["valid"].as_bool() != Some(true) {
            let e = &report["errors"][0];
            return Err(IntelError::new(
                e["code"].as_str().unwrap_or("PLAN_INVALID").to_owned(),
                e["message"]
                    .as_str()
                    .unwrap_or("plan rejected by the editor")
                    .to_owned(),
            ));
        }
        let plan_digest = sha_hex(serde_json::to_string(&plan).unwrap_or_default().as_bytes());
        let diff_digest = sha_hex(
            serde_json::to_string(&report["diff"])
                .unwrap_or_default()
                .as_bytes(),
        );
        let token = format!(
            "pt_{}",
            &sha_hex(format!("{plan_digest}:{}", actor.id).as_bytes())[..32]
        );
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                token.clone(),
                Pending {
                    plan,
                    actor: actor.clone(),
                },
            );
        Ok(json!({
            "plan_token": token, "already_applied": already, "base_revision": revision,
            "plan_digest": plan_digest, "diff_digest": diff_digest,
            "op_count": translated.operations.len(), "refs": {}, "diff": report["diff"],
            "bookkeeping": translated.bookkeeping,
        }))
    }

    fn apply(&self, actor: &Actor, token: &str) -> IntelResult<Value> {
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(token)
            .ok_or_else(|| {
                IntelError::new("PLAN_UNKNOWN", "plan token is unknown or already used")
            })?;
        if pending.actor != *actor {
            return Err(IntelError::new(
                "NOT_ALLOWED",
                "plan token is bound to another actor",
            ));
        }
        let plan_id = pending.plan["planId"].as_str().unwrap_or("").to_owned();
        let reply = surface_result(self.call("apply_plan", json!({"plan": pending.plan}))?)?;
        self.applied_plans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(plan_id);
        Ok(json!({
            "revision": reply["receipt"]["revisionAfter"],
            "status": reply["status"],
            "entry_id": reply["receipt"]["undoReceipt"]["historyIndex"],
            "receipt": reply["receipt"],
        }))
    }

    fn project_path(&self) -> Option<PathBuf> {
        None
    }

    fn toolchain(&self) -> Option<MediaToolchain> {
        None
    }

    fn revision(&self) -> IntelResult<u64> {
        Ok(self.project()?["project"]["revision"].as_u64().unwrap_or(0))
    }

    fn import_begin(&self, _path: &std::path::Path) -> IntelResult<String> {
        Err(IntelError::new(
            "NOT_SUPPORTED",
            "asset import over the bridge is a Phase 2B capability",
        ))
    }

    fn import_poll(&self, _ticket_id: &str) -> IntelResult<Value> {
        Err(IntelError::new(
            "NOT_SUPPORTED",
            "asset import over the bridge is a Phase 2B capability",
        ))
    }

    fn import_cancel(&self, _ticket_id: &str) -> IntelResult<bool> {
        Ok(false)
    }
}
