//! Handlers das operações do catálogo. **Só** falam com a Engine API (`capia-editor-api::Session` e
//! o serviço `ai.*`): escrita no documento é `preview → apply_plan` com `Actor::Api` (um por
//! token); nada de SQL do projeto, nada de caminho do cliente, nada de shell. Respostas nunca
//! carregam caminhos de mídia do usuário nem segredos.

use crate::auth::{self, now_ms, public_view};
use crate::catalog::OpDef;
use crate::core::{API_VERSION, CallCtx, Core, SERVER_VERSION};
use crate::error::{ApiErr, ApiResult};
use crate::mac::{random_hex, sha256_hex};
use crate::scope::{Scope, parse_scopes};
use capia_secrets::{CredentialRef, SecretString};
use capia_store::{ExportRow, ProjectRow, WebhookRow};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

const MAX_SEQUENCE_BYTES: usize = 8 << 20;

fn str_of<'a>(p: &'a Value, k: &str) -> &'a str {
    p[k].as_str().unwrap_or_default()
}

fn page_limit(p: &Value, default: u64, max: u64) -> usize {
    usize::try_from(p["limit"].as_u64().unwrap_or(default).clamp(1, max)).unwrap_or(100)
}

/// Fatia `items` (já ordenados por `key`) depois do cursor `after`, com no máximo `limit`.
fn paginate(
    items: Vec<Value>,
    key: &str,
    after: Option<&str>,
    limit: usize,
) -> (Vec<Value>, Option<String>) {
    let mut it: Vec<Value> = match after {
        Some(a) => items
            .into_iter()
            .skip_while(|v| v[key].as_str().is_none_or(|k| k <= a))
            .collect(),
        None => items,
    };
    let next = if it.len() > limit {
        it.truncate(limit);
        it.last().and_then(|v| v[key].as_str()).map(str::to_owned)
    } else {
        None
    };
    (it, next)
}

/// Remove caminhos locais de um valor (recursivo): a API nunca revela o filesystem do usuário.
fn strip_paths(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for k in ["path", "resolved_path", "location_path", "source_path"] {
                m.remove(k);
            }
            for x in m.values_mut() {
                strip_paths(x);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(strip_paths),
        _ => {}
    }
}

fn webhook_ref(id: &str) -> ApiResult<CredentialRef> {
    CredentialRef::new(format!("capia/server/webhook/{id}"))
        .map_err(|_| ApiErr::internal("invalid webhook id"))
}

fn webhook_view(c: &Core, w: &WebhookRow) -> Value {
    let configured = webhook_ref(&w.id)
        .ok()
        .is_some_and(|r| c.cfg.secrets.exists(&r).unwrap_or(false));
    json!({
        "id": w.id, "url": w.url, "events": w.events, "enabled": w.enabled,
        "description": w.description, "created_ms": w.created_ms, "updated_ms": w.updated_ms,
        "secret_configured": configured,
    })
}

fn valid_events(events: &[Value]) -> ApiResult<Vec<String>> {
    let mut out = Vec::new();
    for e in events {
        let e = e.as_str().unwrap_or_default();
        if e != "*" && !crate::catalog::EVENT_TYPES.contains(&e) {
            return Err(ApiErr::invalid(format!(
                "unknown event type `{e}`; valid: * or {}",
                crate::catalog::EVENT_TYPES.join(", ")
            )));
        }
        if !out.iter().any(|x| x == e) {
            out.push(e.to_owned());
        }
    }
    Ok(out)
}

impl Core {
    #[allow(clippy::too_many_lines)]
    pub fn handle(&self, ctx: &CallCtx, def: &'static OpDef, p: Value) -> ApiResult<Value> {
        match def.name {
            "server.health" => Ok(json!({
                "status": if self.shutting_down.load(std::sync::atomic::Ordering::SeqCst) { "shutting_down" } else { "ok" },
                "version": SERVER_VERSION,
                "api_version": API_VERSION,
                "uptime_ms": u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            })),
            "server.info" => self.op_info(),
            "server.metrics" => {
                use std::sync::atomic::Ordering::Relaxed;
                let m = &self.metrics;
                Ok(json!({
                    "uptime_ms": u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    "requests": m.requests.load(Relaxed), "writes": m.writes.load(Relaxed),
                    "errors_4xx": m.errors_4xx.load(Relaxed), "errors_5xx": m.errors_5xx.load(Relaxed),
                    "rate_limited": m.rate_limited.load(Relaxed), "idempotent_replays": m.replays.load(Relaxed),
                    "inflight": self.inflight.load(Relaxed), "uploads_active": self.uploads_active.load(Relaxed),
                    "sse_streams": self.sse_active.load(Relaxed),
                    "webhook_deliveries_pending": self.db.deliveries_pending_count().unwrap_or(0),
                }))
            }
            // ---- tokens ---------------------------------------------------------------------
            "tokens.create" => self.op_token_create(ctx, &p),
            "tokens.list" => Ok(json!({"tokens": self.db.token_list()?.iter().map(public_view).collect::<Vec<_>>()})),
            "tokens.revoke" => {
                let id = str_of(&p, "token_id");
                if self.db.token_get(id)?.is_none() {
                    return Err(ApiErr::not_found("unknown token"));
                }
                Ok(json!({"revoked": self.db.token_revoke(id, now_ms())?, "token_id": id}))
            }
            "tokens.rotate" => self.op_token_rotate(ctx, &p),
            "audit.list" => {
                let rows = self
                    .db
                    .audit_list(i64::try_from(p["after"].as_u64().unwrap_or(0)).unwrap_or(0), u32::try_from(page_limit(&p, 100, 500)).unwrap_or(100))?;
                let next = rows.last().map(|r| r.seq);
                Ok(json!({"entries": rows, "next": next}))
            }
            // ---- projetos -------------------------------------------------------------------
            "projects.create" => self.op_project_create(&p),
            "projects.list" => {
                let limit = page_limit(&p, 50, 200);
                let rows = self.db.project_list(p["after"].as_str(), u32::try_from(limit + 1).unwrap_or(51))?;
                let open = self.open_project_id();
                let items: Vec<Value> = rows.iter().map(|r| project_view(r, open.as_deref())).collect();
                let (items, next) = paginate(items, "id", None, limit);
                Ok(json!({"projects": items, "next": next}))
            }
            "projects.get" => {
                let id = str_of(&p, "project_id");
                let row = self
                    .db
                    .project_get(id)?
                    .ok_or_else(|| ApiErr::not_found(format!("project `{id}` does not exist")))?;
                Ok(json!({"project": project_view(&row, self.open_project_id().as_deref())}))
            }
            "projects.open" => self.op_project_open(&p),
            "projects.close" => {
                self.switch_gate()?;
                self.session_call("project.close", json!({}))?;
                self.set_open_project(None);
                Ok(json!({"closed": true}))
            }
            "projects.summary" => self.op_project_summary(&p),
            // ---- uploads + assets -----------------------------------------------------------
            "uploads.create_inline" => {
                let v = self.upload_inline(
                    ctx,
                    str_of(&p, "filename"),
                    str_of(&p, "content_base64"),
                    p["expected_sha256"].as_str(),
                )?;
                Ok(json!({"upload": v}))
            }
            "uploads.list" => {
                let limit = page_limit(&p, 50, 200);
                let (items, next) = paginate(self.list_uploads(), "upload_id", p["after"].as_str(), limit);
                Ok(json!({"uploads": items, "next": next}))
            }
            "uploads.delete" => {
                let id = str_of(&p, "upload_id");
                if self.read_upload_meta(id).is_none() {
                    return Err(ApiErr::not_found("unknown upload"));
                }
                Ok(json!({"deleted": self.delete_upload(id), "upload_id": id}))
            }
            "assets.import" => self.op_asset_import(&p),
            "assets.list" => {
                let mut rows = self.session_call("assets.list", json!({}))?;
                strip_paths(&mut rows);
                let mut items = rows.as_array().cloned().unwrap_or_default();
                items.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
                let (items, next) = paginate(items, "id", p["after"].as_str(), page_limit(&p, 100, 200));
                Ok(json!({"assets": items, "next": next}))
            }
            "assets.get" => {
                let id = str_of(&p, "asset_id");
                let mut rows = self.session_call("assets.list", json!({}))?;
                strip_paths(&mut rows);
                rows.as_array()
                    .and_then(|a| a.iter().find(|r| r["id"] == id))
                    .map(|r| json!({"asset": r}))
                    .ok_or_else(|| ApiErr::not_found(format!("asset `{id}` does not exist")))
            }
            "imports.get" => {
                let ticket = str_of(&p, "ticket_id");
                let mut v = self.lock_session().agent_import_poll(ticket)?;
                strip_paths(&mut v);
                Ok(json!({"import": v}))
            }
            // ---- timeline -------------------------------------------------------------------
            "sequences.list" => {
                let snap = self.session_call("project.snapshot", json!({}))?;
                Ok(json!({"revision": snap["revision"], "sequences": snap["sequences"]}))
            }
            "sequences.get" => {
                let v = self.session_call("sequence.get", json!({"sequence": str_of(&p, "sequence_id")}))?;
                if v.to_string().len() > MAX_SEQUENCE_BYTES {
                    return Err(ApiErr::new(
                        413,
                        "RESPONSE_TOO_LARGE",
                        "the sequence is too large for one response; page it with timeline.query",
                    ));
                }
                Ok(json!({"revision": self.lock_session().revision(), "sequence": v}))
            }
            "timeline.query" => self.op_timeline_query(&p),
            "commands.preview" => self.op_preview(ctx, &p),
            "commands.apply" => {
                let actor = Self::actor_of(ctx)?;
                let v = self.lock_session().agent_apply(&actor, str_of(&p, "plan_token"))?;
                Ok(v)
            }
            "history.list" => {
                let v = self.session_call("history.list", json!({}))?;
                let after = p["after"].as_u64().unwrap_or(0);
                let limit = page_limit(&p, 100, 500);
                let entries: Vec<Value> = v["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|e| e["id"].as_u64().unwrap_or(0) > after)
                    .take(limit + 1)
                    .cloned()
                    .collect();
                let next = (entries.len() > limit).then(|| entries[limit - 1]["id"].clone());
                Ok(json!({"entries": entries.into_iter().take(limit).collect::<Vec<_>>(), "cursor": v["cursor"], "next": next}))
            }
            // ---- runs -----------------------------------------------------------------------
            "runs.create" => self.op_run_create(ctx, &p),
            "runs.list" => {
                let v = self.ai_call("ai.run.list", json!({"limit": p["limit"].as_u64().unwrap_or(100)}))?;
                Ok(v)
            }
            "runs.get" => {
                let mut v = self.ai_call("ai.run.get", json!({"run_id": str_of(&p, "run_id")}))?;
                scrub_run(&mut v);
                Ok(v)
            }
            "runs.pause" => self.ai_call("ai.run.pause", json!({"run_id": str_of(&p, "run_id")})),
            "runs.resume" => self.ai_call("ai.run.resume", json!({"run_id": str_of(&p, "run_id")})),
            "runs.cancel" => self.ai_call("ai.run.cancel", json!({"run_id": str_of(&p, "run_id")})),
            "runs.approve" => {
                let by = format!(
                    "api:{}",
                    ctx.principal.as_ref().map(|p| p.token_id.as_str()).unwrap_or("?")
                );
                self.ai_call(
                    "ai.run.decide",
                    json!({
                        "run_id": str_of(&p, "run_id"), "decision_id": str_of(&p, "decision_id"),
                        "option": str_of(&p, "option"), "payload": p.get("payload").cloned().unwrap_or(Value::Null),
                        "decided_by": by,
                    }),
                )
            }
            "runs.plan" => {
                let v = self.ai_call("ai.run.get", json!({"run_id": str_of(&p, "run_id")}))?;
                Ok(json!({
                    "run_id": str_of(&p, "run_id"),
                    "production_plan": v["run"]["production_plan"],
                    "edit_plans": v["run"]["edit_plans"],
                    "validation": v["run"]["validation"],
                    "applied": v["run"]["applied"],
                    "pending": v["run"]["pending"],
                }))
            }
            "runs.review" => {
                let v = self.ai_call("ai.run.get", json!({"run_id": str_of(&p, "run_id")}))?;
                let stages: Vec<Value> = v["stages"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|s| s["stage"] == "review" || s["stage"] == "correct")
                    .cloned()
                    .collect();
                Ok(json!({"run_id": str_of(&p, "run_id"), "reviews": v["run"]["reviews"], "report": v["run"]["report"], "stages": stages}))
            }
            "runs.cost" => {
                let v = self.ai_call("ai.run.get", json!({"run_id": str_of(&p, "run_id")}))?;
                Ok(json!({"run_id": str_of(&p, "run_id"), "usage": v["usage"], "budget": v["run"]["budget"]}))
            }
            "runs.events" => {
                let v = self.ai_call(
                    "ai.run.events",
                    json!({"run_id": str_of(&p, "run_id"), "after": p["after"].as_u64().unwrap_or(0)}),
                )?;
                let next = v["events"].as_array().and_then(|a| a.last()).map(|e| e["seq"].clone());
                Ok(json!({"events": v["events"], "next": next}))
            }
            "runs.variants" => self.ai_call(
                "ai.run.variants",
                json!({"run_id": str_of(&p, "run_id"), "count": p["count"], "axis": p.get("axis").cloned().unwrap_or(json!([]))}),
            ),
            "memory.list" => self.ai_call("ai.memory.list", json!({})),
            "gateway.status" => self.ai_call("ai.gateway.status", json!({})),
            // ---- exports --------------------------------------------------------------------
            "exports.start" => self.op_export_start(&p),
            "exports.list" | "deliverables.list" => {
                let limit = page_limit(&p, 50, 200);
                let rows = self.db.export_list(
                    Some(str_of(&p, "project_id")),
                    p["after"].as_str().unwrap_or(""),
                    u32::try_from(limit + 1).unwrap_or(51),
                )?;
                let only_done = def.name == "deliverables.list";
                let items: Vec<Value> = rows
                    .iter()
                    .filter(|r| !only_done || r.state == "completed")
                    .map(export_view)
                    .collect();
                let (items, next) = paginate(items, "id", None, limit);
                Ok(json!({"exports": items, "next": next}))
            }
            "exports.get" => {
                let id = str_of(&p, "export_id");
                let row = self
                    .db
                    .export_get(id)?
                    .filter(|r| r.project_id == str_of(&p, "project_id"))
                    .ok_or_else(|| ApiErr::not_found(format!("export `{id}` does not exist")))?;
                Ok(json!({"export": export_view(&row)}))
            }
            "exports.cancel" => {
                let id = str_of(&p, "export_id");
                let row = self
                    .db
                    .export_get(id)?
                    .filter(|r| r.project_id == str_of(&p, "project_id"))
                    .ok_or_else(|| ApiErr::not_found(format!("export `{id}` does not exist")))?;
                let r = self.session_call("export.cancel", json!({"id": row.batch}))?;
                Ok(json!({"cancel_requested": r["cancelled"], "export_id": id}))
            }
            // ---- webhooks -------------------------------------------------------------------
            "webhooks.create" => self.op_webhook_create(&p),
            "webhooks.list" => {
                let rows = self.db.webhook_list()?;
                Ok(json!({"webhooks": rows.iter().map(|w| webhook_view(self, w)).collect::<Vec<_>>()}))
            }
            "webhooks.get" => {
                let w = self.webhook(str_of(&p, "webhook_id"))?;
                Ok(json!({"webhook": webhook_view(self, &w)}))
            }
            "webhooks.update" => self.op_webhook_update(&p),
            "webhooks.rotate_secret" => {
                let mut w = self.webhook(str_of(&p, "webhook_id"))?;
                let secret = self.store_webhook_secret(&w.id)?;
                w.updated_ms = now_ms();
                self.db.webhook_update(&w)?;
                // entregas que morreram por falta do segredo voltam à fila
                let n = self.requeue_secret_unavailable(&w.id);
                Ok(json!({"webhook": webhook_view(self, &w), "secret": secret, "requeued": n}))
            }
            "webhooks.delete" => {
                let id = str_of(&p, "webhook_id");
                self.webhook(id)?;
                let _ = webhook_ref(id).map(|r| self.cfg.secrets.delete(&r));
                Ok(json!({"deleted": self.db.webhook_delete(id)?, "webhook_id": id}))
            }
            "webhooks.test" => {
                let w = self.webhook(str_of(&p, "webhook_id"))?;
                let event_id = format!("test.{}", random_hex(8).map_err(ApiErr::internal)?);
                self.db.event_append(
                    &event_id,
                    "webhook.test",
                    now_ms(),
                    None,
                    None,
                    None,
                    &json!({"message": "this is a test delivery from CapIA", "webhook_id": w.id}),
                    Some(&w.id),
                )?;
                self.wake.notify();
                Ok(json!({"queued": true, "event_id": event_id}))
            }
            "webhooks.deliveries" => {
                let id = str_of(&p, "webhook_id");
                self.webhook(id)?;
                let limit = page_limit(&p, 50, 200);
                let rows = self.db.deliveries_list(
                    Some(id),
                    i64::try_from(p["after"].as_u64().unwrap_or(0)).unwrap_or(0),
                    u32::try_from(limit).unwrap_or(50),
                )?;
                let next = (rows.len() == limit).then(|| rows.last().map(|r| r.id)).flatten();
                Ok(json!({"deliveries": rows, "next": next}))
            }
            "webhooks.redeliver" => {
                let id = str_of(&p, "webhook_id");
                self.webhook(id)?;
                let did = i64::try_from(p["delivery_id"].as_u64().unwrap_or(0)).unwrap_or(0);
                let d = self
                    .db
                    .delivery_get(did)?
                    .filter(|d| d.webhook_id == id)
                    .ok_or_else(|| ApiErr::not_found("unknown delivery"))?;
                let ok = self.db.delivery_requeue(d.id, now_ms())?;
                if !ok {
                    return Err(ApiErr::conflict(
                        "DELIVERY_IN_FLIGHT",
                        "this delivery is pending or being delivered",
                    ));
                }
                self.wake.notify();
                Ok(json!({"queued": true, "delivery_id": d.id}))
            }
            // ---- eventos --------------------------------------------------------------------
            "events.list" => {
                let limit = page_limit(&p, 100, 500);
                let rows = self.db.events_after(
                    i64::try_from(p["after"].as_u64().unwrap_or(0)).unwrap_or(0),
                    u32::try_from(limit).unwrap_or(100),
                )?;
                let next = rows.last().map(|e| e.seq);
                Ok(json!({"events": rows, "next": next}))
            }
            other => Err(ApiErr::new(
                501,
                "NOT_IMPLEMENTED",
                format!("operation `{other}` has no handler"),
            )),
        }
    }

    // ---- info / tokens -----------------------------------------------------------------------

    fn op_info(&self) -> ApiResult<Value> {
        let engine = self
            .session_call("engine.info", json!({}))
            .unwrap_or(Value::Null);
        let ai_enabled = self
            .ai_call("ai.status", json!({}))
            .ok()
            .and_then(|v| v["enabled"].as_bool());
        Ok(json!({
            "version": SERVER_VERSION,
            "api_version": API_VERSION,
            "engine": engine,
            "ai_enabled": ai_enabled,
            "project_open": self.open_project_id(),
            "shutting_down": self.shutting_down.load(std::sync::atomic::Ordering::SeqCst),
            "limits": {
                "max_json_bytes": self.cfg.max_json_bytes,
                "max_upload_bytes": self.cfg.max_upload_bytes,
                "upload_quota_bytes": self.cfg.upload_quota_bytes,
                "max_inline_upload_bytes": crate::uploads::MAX_INLINE_BYTES,
            },
            "loopback_only": self.cfg.is_loopback(),
        }))
    }

    fn op_token_create(&self, ctx: &CallCtx, p: &Value) -> ApiResult<Value> {
        let caller = ctx
            .principal
            .as_ref()
            .ok_or_else(|| ApiErr::unauthorized("authentication required"))?;
        let raw: Vec<String> = p["scopes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect();
        let scopes = parse_scopes(&raw).map_err(ApiErr::invalid)?;
        // nenhum token concede o que ele próprio não tem (sem escalada de privilégio)
        let escalated: Vec<&str> = scopes
            .iter()
            .filter(|s| !caller.has(**s))
            .map(|s| s.as_str())
            .collect();
        if !escalated.is_empty() {
            return Err(ApiErr::forbidden(
                "SCOPE_ESCALATION",
                format!(
                    "a token cannot grant scopes it does not hold: {}",
                    escalated.join(", ")
                ),
            ));
        }
        let now = now_ms();
        let (row, secret) = auth::mint(
            str_of(p, "name"),
            &scopes,
            p["expires_in_seconds"]
                .as_u64()
                .map(|s| now.saturating_add(s * 1000)),
            None,
            now,
        )?;
        self.db.token_insert(&row)?;
        Ok(
            json!({"token": public_view(&row), "secret": secret, "note": "store the secret now: it is shown only once"}),
        )
    }

    fn op_token_rotate(&self, ctx: &CallCtx, p: &Value) -> ApiResult<Value> {
        let caller = ctx
            .principal
            .as_ref()
            .ok_or_else(|| ApiErr::unauthorized("authentication required"))?;
        let old = self
            .db
            .token_get(str_of(p, "token_id"))?
            .ok_or_else(|| ApiErr::not_found("unknown token"))?;
        if old.revoked_ms.is_some() {
            return Err(ApiErr::conflict(
                "TOKEN_REVOKED",
                "the token is already revoked",
            ));
        }
        let scopes = parse_scopes(&old.scopes).map_err(ApiErr::internal)?;
        let escalated: Vec<&str> = scopes
            .iter()
            .filter(|s| !caller.has(**s))
            .map(|s| s.as_str())
            .collect();
        if !escalated.is_empty() {
            return Err(ApiErr::forbidden(
                "SCOPE_ESCALATION",
                format!(
                    "rotating this token would mint scopes the caller does not hold: {}",
                    escalated.join(", ")
                ),
            ));
        }
        let now = now_ms();
        let ttl = old.expires_ms.map(|e| e.saturating_sub(old.created_ms));
        let (row, secret) = auth::mint(
            &old.name,
            &scopes,
            ttl.map(|t| now + t),
            Some(old.id.clone()),
            now,
        )?;
        if !self.db.token_rotate(&old.id, &row, now)? {
            return Err(ApiErr::conflict(
                "TOKEN_REVOKED",
                "the token was revoked concurrently",
            ));
        }
        Ok(json!({"token": public_view(&row), "secret": secret, "revoked_id": old.id}))
    }

    // ---- projetos ----------------------------------------------------------------------------

    /// Trocar/fechar o projeto aberto só com a Run/exports ociosos (nada é cortado no meio).
    pub fn switch_gate(&self) -> ApiResult<()> {
        if self.open_project_id().is_none() {
            return Ok(());
        }
        let running = self
            .ai_call("ai.run.list", json!({"limit": 200}))
            .ok()
            .and_then(|v| {
                v["runs"]
                    .as_array()
                    .map(|a| a.iter().filter(|r| r["status"] == "running").count())
            })
            .unwrap_or(0);
        let exporting = self
            .db
            .export_list(None, "", 500)?
            .iter()
            .filter(|e| e.state == "queued" || e.state == "running")
            .count();
        if running > 0 || exporting > 0 {
            return Err(ApiErr::conflict(
                "PROJECT_BUSY",
                format!(
                    "the open project has {running} running run(s) and {exporting} active export(s); wait or cancel them"
                ),
            ));
        }
        Ok(())
    }

    fn op_project_create(&self, p: &Value) -> ApiResult<Value> {
        self.switch_gate()?;
        let id = format!("prj_{}", random_hex(8).map_err(ApiErr::internal)?);
        let dir = self.cfg.data_dir.join("projects").join(&id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| ApiErr::unavailable("STORAGE_UNAVAILABLE", e.to_string()))?;
        let path = dir.join("project.capia");
        if let Err(e) = self.session_call(
            "project.create",
            json!({"path": path.display().to_string()}),
        ) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
        let row = ProjectRow {
            id: id.clone(),
            name: str_of(p, "name").to_owned(),
            path: path.display().to_string(),
            created_ms: now_ms(),
            last_opened_ms: Some(now_ms()),
        };
        self.db.project_insert(&row)?;
        self.set_open_project(Some(id));
        Ok(
            json!({"project": project_view(&row, Some(&row.id)), "revision": self.lock_session().revision()}),
        )
    }

    fn op_project_open(&self, p: &Value) -> ApiResult<Value> {
        let id = str_of(p, "project_id");
        let row = self
            .db
            .project_get(id)?
            .ok_or_else(|| ApiErr::not_found(format!("project `{id}` does not exist")))?;
        if self.open_project_id().as_deref() != Some(id) {
            self.switch_gate()?;
            self.session_call("project.open", json!({"path": row.path}))?;
            self.set_open_project(Some(id.to_owned()));
        }
        self.db.project_touch_open(id, now_ms())?;
        Ok(
            json!({"project": project_view(&row, Some(id)), "revision": self.lock_session().revision()}),
        )
    }

    fn op_project_summary(&self, p: &Value) -> ApiResult<Value> {
        let id = str_of(p, "project_id");
        let row = self
            .db
            .project_get(id)?
            .ok_or_else(|| ApiErr::not_found("unknown project"))?;
        let snap = self.session_call("project.snapshot", json!({}))?;
        let seqs = snap["sequences"].as_array().cloned().unwrap_or_default();
        let clips: u64 = seqs.iter().filter_map(|s| s["clip_count"].as_u64()).sum();
        let mut runs = Map::new();
        if let Ok(v) = self.ai_call("ai.run.list", json!({"limit": 200})) {
            for r in v["runs"].as_array().into_iter().flatten() {
                let st = r["status"].as_str().unwrap_or("unknown").to_owned();
                let n = runs.get(&st).and_then(Value::as_u64).unwrap_or(0) + 1;
                runs.insert(st, json!(n));
            }
        }
        Ok(json!({
            "project": project_view(&row, Some(id)),
            "revision": snap["revision"], "can_undo": snap["can_undo"], "can_redo": snap["can_redo"],
            "sequences": seqs.len(), "clips": clips,
            "assets": snap["assets"].as_array().map_or(0, Vec::len),
            "runs": runs,
        }))
    }

    // ---- assets ------------------------------------------------------------------------------

    fn op_asset_import(&self, p: &Value) -> ApiResult<Value> {
        let upload_id = str_of(p, "upload_id");
        let (blob, mut meta) = self
            .upload_blob(upload_id)
            .ok_or_else(|| ApiErr::not_found(format!("upload `{upload_id}` does not exist")))?;
        if meta["state"] != "staged" {
            return Err(ApiErr::conflict(
                "UPLOAD_CONSUMED",
                "this upload was already imported",
            ));
        }
        let kind = meta["kind"].as_str().unwrap_or_default();
        if kind != "video" && kind != "audio" && kind != "image" {
            return Err(ApiErr::invalid(
                "only video, audio and image uploads can be imported as assets",
            ));
        }
        // a mídia sai do staging para a pasta durável do projeto (o staging é descartável)
        let project_dir = PathBuf::from(
            self.db
                .project_get(str_of(p, "project_id"))?
                .map(|r| r.path)
                .unwrap_or_default(),
        )
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| ApiErr::internal("project path has no parent"))?;
        let media_dir = project_dir.join("media").join(upload_id);
        std::fs::create_dir_all(&media_dir)
            .map_err(|e| ApiErr::unavailable("STORAGE_UNAVAILABLE", e.to_string()))?;
        let dest = media_dir.join(meta["filename"].as_str().unwrap_or("media"));
        std::fs::rename(&blob, &dest)
            .or_else(|_| {
                // outro volume: copia e remove
                std::fs::copy(&blob, &dest)
                    .map(|_| ())
                    .and_then(|()| std::fs::remove_file(&blob))
            })
            .map_err(|e| ApiErr::unavailable("STORAGE_UNAVAILABLE", e.to_string()))?;
        let ticket = self.lock_session().agent_import_begin(&dest)?;
        meta["state"] = json!("imported");
        meta["import_ticket"] = json!(ticket);
        let _ = self.write_upload_meta(upload_id, &meta);
        Ok(json!({"ticket_id": ticket, "upload_id": upload_id, "state": "queued"}))
    }

    // ---- timeline ----------------------------------------------------------------------------

    fn op_timeline_query(&self, p: &Value) -> ApiResult<Value> {
        let sid = str_of(p, "sequence_id");
        let seq = self.session_call("sequence.get", json!({"sequence": sid}))?;
        let digest = sha256_hex(seq["clips"].to_string().as_bytes());
        let from = p["from_ticks"].as_i64();
        let to = p["to_ticks"].as_i64();
        let track = p["track"].as_str();
        let mut clips: Vec<(i64, String, Value)> = seq["clips"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(id, c)| {
                let start = c["start"].as_i64()?;
                let end = start.saturating_add(c["duration"].as_i64().unwrap_or(0));
                if from.is_some_and(|f| end <= f) || to.is_some_and(|t| start >= t) {
                    return None;
                }
                if track.is_some_and(|t| c["track"].as_str() != Some(t)) {
                    return None;
                }
                let mut c = c.clone();
                c["id"] = json!(id);
                Some((start, id.clone(), c))
            })
            .collect();
        clips.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        let total = clips.len();
        let limit = page_limit(p, 100, 500);
        let after = p["after"].as_str();
        let mut iter: Vec<&(i64, String, Value)> = match after {
            Some(a) => {
                let pos = clips.iter().position(|c| c.1 == a);
                match pos {
                    Some(i) => clips.iter().skip(i + 1).collect(),
                    None => return Err(ApiErr::invalid("`after` is not a clip of this query")),
                }
            }
            None => clips.iter().collect(),
        };
        let more = iter.len() > limit;
        iter.truncate(limit);
        let next = more.then(|| iter.last().map(|c| c.1.clone())).flatten();
        Ok(json!({
            "sequence": sid, "revision": self.lock_session().revision(), "digest": digest,
            "total": total, "clips": iter.iter().map(|c| c.2.clone()).collect::<Vec<_>>(), "next": next,
        }))
    }

    fn op_preview(&self, ctx: &CallCtx, p: &Value) -> ApiResult<Value> {
        let actor = Self::actor_of(ctx)?;
        let label = p["label"].as_str().unwrap_or("api").to_owned();
        let v = self
            .lock_session()
            .agent_preview(&actor, &label, p["commands"].clone())?;
        // concorrência otimista: o cliente diz sobre qual revisão raciocinou
        if let Some(exp) = p["expected_revision"].as_u64() {
            let base = v["base_revision"].as_u64().unwrap_or(0);
            if exp != base {
                return Err(ApiErr::conflict(
                    "REVISION_CONFLICT",
                    format!(
                        "the document is at revision {base}, not {exp}; re-read and preview again"
                    ),
                )
                .with_details(json!({"expected_revision": exp, "actual_revision": base})));
            }
        }
        Ok(v)
    }

    // ---- runs --------------------------------------------------------------------------------

    fn op_run_create(&self, ctx: &CallCtx, p: &Value) -> ApiResult<Value> {
        // documentos chegam como ids de upload; o servidor resolve o caminho (o cliente nunca manda um)
        let mut documents = Vec::new();
        for d in p["documents"].as_array().into_iter().flatten() {
            let id = d.as_str().unwrap_or_default();
            let (blob, meta) = self
                .upload_blob(id)
                .ok_or_else(|| ApiErr::not_found(format!("upload `{id}` does not exist")))?;
            if meta["kind"] != "document" {
                return Err(ApiErr::invalid(format!("upload `{id}` is not a document")));
            }
            documents.push(json!(blob.display().to_string()));
        }
        let mut inputs = Map::new();
        for k in [
            "brief_text",
            "assets",
            "references",
            "note",
            "deliverables",
            "variants",
            "client_id",
        ] {
            if let Some(v) = p.get(k).filter(|v| !v.is_null()) {
                inputs.insert(k.to_owned(), v.clone());
            }
        }
        inputs.insert("documents".into(), Value::Array(documents));
        // só ajustes que NÃO ampliam poder: a política padrão exige aprovação do plano
        let mut policy = Map::new();
        if let Some(po) = p["policy"].as_object() {
            for (k, v) in po {
                match k.as_str() {
                    "final_approval" | "critic_vision" | "allow_generation" | "allow_gateway" => {
                        if v.is_boolean() {
                            policy.insert(k.clone(), v.clone());
                        }
                    }
                    "critic_max_frames" | "max_question_rounds" if v.is_u64() => {
                        policy.insert(k.clone(), v.clone());
                    }
                    other => {
                        return Err(ApiErr::invalid(format!(
                            "policy field `{other}` cannot be set through the API"
                        )));
                    }
                }
            }
        }
        let mut budget = Map::new();
        if let Some(b) = p["budget"].as_object() {
            for (k, v) in b {
                if matches!(
                    k.as_str(),
                    "max_cost_micros"
                        | "max_tokens"
                        | "max_provider_calls"
                        | "max_generations"
                        | "max_review_loops"
                        | "max_replans"
                        | "max_wall_time_ms"
                ) && v.is_u64()
                {
                    budget.insert(k.clone(), v.clone());
                } else {
                    return Err(ApiErr::invalid(format!("budget field `{k}` is not valid")));
                }
            }
        }
        let _ = ctx;
        let mut v = self.ai_call(
            "ai.run.create",
            json!({
                "inputs": inputs, "policy": policy, "budget": budget,
                "start": p["start"].as_bool().unwrap_or(true),
            }),
        )?;
        scrub_run(&mut v);
        Ok(v)
    }

    // ---- exports -----------------------------------------------------------------------------

    fn op_export_start(&self, p: &Value) -> ApiResult<Value> {
        let project_id = str_of(p, "project_id");
        let snap = self.session_call("project.snapshot", json!({}))?;
        let known: Vec<&str> = snap["sequences"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s["id"].as_str())
            .collect();
        let mut items = Vec::new();
        let mut rows = Vec::new();
        let now = now_ms();
        for it in p["items"].as_array().into_iter().flatten() {
            let sequence = str_of(it, "sequence");
            if !known.contains(&sequence) {
                return Err(ApiErr::not_found(format!(
                    "sequence `{sequence}` does not exist"
                )));
            }
            let id = format!("exp_{}", random_hex(6).map_err(ApiErr::internal)?);
            let preset = it["preset"].as_str().unwrap_or("h264-mp4");
            let stem = crate::uploads::sanitize_filename(it["name"].as_str().unwrap_or(sequence));
            let stem = stem
                .rsplit_once('.')
                .map_or(stem.clone(), |(s, _)| s.to_owned());
            let ext = if preset == "intermediate" {
                "mov"
            } else {
                "mp4"
            };
            let dir = self.cfg.data_dir.join("exports").join(project_id).join(&id);
            std::fs::create_dir_all(&dir)
                .map_err(|e| ApiErr::unavailable("STORAGE_UNAVAILABLE", e.to_string()))?;
            let path = dir.join(format!("{stem}.{ext}"));
            let mut item = json!({"id": id, "sequence": sequence, "preset": preset, "path": path.display().to_string()});
            for k in ["width", "height", "encoder"] {
                if let Some(v) = it.get(k).filter(|v| !v.is_null()) {
                    item[k] = v.clone();
                }
            }
            rows.push(ExportRow {
                id: id.clone(),
                project_id: project_id.to_owned(),
                batch: "pending".into(),
                sequence: sequence.to_owned(),
                preset: preset.to_owned(),
                state: "queued".into(),
                path: Some(path.display().to_string()),
                report: None,
                error: None,
                created_ms: now,
                finished_ms: None,
            });
            items.push(item);
        }
        for r in &rows {
            self.db.export_insert(r)?;
        }
        let started = self.session_call("export.start", json!({"items": items}));
        match started {
            Ok(v) => {
                let batch = v["batch"].as_str().unwrap_or_default().to_owned();
                for r in &rows {
                    let _ = self.db.export_set_batch(&r.id, &batch);
                }
                Ok(
                    json!({"batch": batch, "exports": rows.iter().map(export_view).collect::<Vec<_>>()}),
                )
            }
            Err(e) => {
                for r in &rows {
                    let _ = self.db.export_update(
                        &r.id,
                        "failed",
                        None,
                        Some(&json!({"code": e.code, "message": e.message})),
                        now_ms(),
                    );
                }
                Err(e)
            }
        }
    }

    // ---- webhooks ----------------------------------------------------------------------------

    pub fn webhook(&self, id: &str) -> ApiResult<WebhookRow> {
        self.db
            .webhook_get(id)?
            .ok_or_else(|| ApiErr::not_found(format!("webhook `{id}` does not exist")))
    }

    /// Gera o segredo de assinatura, guarda no cofre (nunca no banco) e devolve o valor **uma vez**.
    fn store_webhook_secret(&self, id: &str) -> ApiResult<String> {
        let secret = format!("whsec_{}", random_hex(32).map_err(ApiErr::internal)?);
        capia_secrets::register_global(&secret);
        self.cfg
            .secrets
            .put(&webhook_ref(id)?, SecretString::new(secret.clone()))
            .map_err(|e| ApiErr::unavailable("SECRET_STORE_UNAVAILABLE", e.to_string()))?;
        Ok(secret)
    }

    fn check_webhook_url(&self, url: &str) -> ApiResult<()> {
        self.webhook_client
            .check_url(url)
            .map(|_| ())
            .map_err(|e| ApiErr::invalid(format!("webhook URL refused: {}", e.message)))
    }

    fn op_webhook_create(&self, p: &Value) -> ApiResult<Value> {
        let url = str_of(p, "url");
        self.check_webhook_url(url)?;
        if self.db.webhook_list()?.len() >= 32 {
            return Err(ApiErr::conflict("TOO_MANY_WEBHOOKS", "at most 32 webhooks"));
        }
        let events = valid_events(p["events"].as_array().map_or(&[][..], Vec::as_slice))?;
        let id = format!("whk_{}", random_hex(6).map_err(ApiErr::internal)?);
        let secret = self.store_webhook_secret(&id)?;
        let now = now_ms();
        let w = WebhookRow {
            id: id.clone(),
            url: url.to_owned(),
            events,
            secret_ref: webhook_ref(&id)?.as_str().to_owned(),
            enabled: true,
            description: p["description"].as_str().unwrap_or_default().to_owned(),
            created_ms: now,
            updated_ms: now,
        };
        self.db.webhook_insert(&w)?;
        Ok(
            json!({"webhook": webhook_view(self, &w), "secret": secret, "note": "store the signing secret now: it is shown only once"}),
        )
    }

    fn op_webhook_update(&self, p: &Value) -> ApiResult<Value> {
        let mut w = self.webhook(str_of(p, "webhook_id"))?;
        if let Some(url) = p["url"].as_str() {
            self.check_webhook_url(url)?;
            url.clone_into(&mut w.url);
        }
        if let Some(ev) = p["events"].as_array() {
            w.events = valid_events(ev)?;
        }
        if let Some(en) = p["enabled"].as_bool() {
            w.enabled = en;
        }
        if let Some(d) = p["description"].as_str() {
            d.clone_into(&mut w.description);
        }
        w.updated_ms = now_ms();
        self.db.webhook_update(&w)?;
        Ok(json!({"webhook": webhook_view(self, &w)}))
    }

    fn requeue_secret_unavailable(&self, webhook_id: &str) -> u64 {
        let mut n = 0;
        let mut after = 0;
        while let Ok(rows) = self.db.deliveries_list(Some(webhook_id), after, 200) {
            if rows.is_empty() {
                break;
            }
            for d in &rows {
                after = d.id;
                if d.state == "dead"
                    && d.last_error
                        .as_deref()
                        .is_some_and(|e| e.starts_with("SECRET_UNAVAILABLE"))
                    && self.db.delivery_requeue(d.id, now_ms()).unwrap_or(false)
                {
                    n += 1;
                }
            }
        }
        if n > 0 {
            self.wake.notify();
        }
        n
    }
}

fn project_view(r: &ProjectRow, open: Option<&str>) -> Value {
    json!({
        "id": r.id, "name": r.name, "created_ms": r.created_ms,
        "last_opened_ms": r.last_opened_ms, "open": open == Some(r.id.as_str()),
    })
}

fn export_view(e: &ExportRow) -> Value {
    json!({
        "id": e.id, "project_id": e.project_id, "batch": e.batch, "sequence": e.sequence,
        "preset": e.preset, "state": e.state, "path": e.path, "report": e.report,
        "error": e.error, "created_ms": e.created_ms, "finished_ms": e.finished_ms,
    })
}

/// A resposta de uma Run nunca carrega caminhos de documentos do servidor.
fn scrub_run(v: &mut Value) {
    for ptr in ["/run/inputs/documents", "/inputs/documents"] {
        if let Some(arr) = v.pointer_mut(ptr).and_then(Value::as_array_mut) {
            for d in arr.iter_mut() {
                let name = d
                    .as_str()
                    .and_then(|s| Path::new(s).file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                *d = json!(name);
            }
        }
    }
    strip_paths(v);
}

/// Scopes de uma operação (para a matriz de testes).
pub fn scope_of(def: &OpDef) -> Option<Scope> {
    def.scope
}
