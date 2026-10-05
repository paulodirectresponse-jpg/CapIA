//! **Catálogo único de operações** (ADR-102): REST, MCP, OpenAPI e a matriz de scopes derivam daqui.
//! Cada operação declara scope, efeito colateral (mutante ou não), classe de rate limit, rota REST e
//! schema JSON dos parâmetros. Não existe rota/tool fora do catálogo: um teste de arquitetura
//! garante que nenhuma entrada fica sem scope (exceto o health público) e que REST e MCP veem o
//! mesmo conjunto — a paridade UI ↔ REST ↔ MCP é por construção, não por convenção.

use crate::scope::Scope;
use serde_json::{Value, json};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    Read,
    Write,
    Upload,
    RunStart,
    Approve,
    Export,
    Admin,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Upload => "upload",
            Self::RunStart => "run_start",
            Self::Approve => "approve",
            Self::Export => "export",
            Self::Admin => "admin",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// REST **e** MCP.
    Both,
    /// Só REST (transporte em streaming que MCP não tem).
    RestOnly,
}

#[derive(Debug)]
pub struct OpDef {
    pub name: &'static str,
    pub summary: &'static str,
    /// `None` apenas no health público.
    pub scope: Option<Scope>,
    pub mutating: bool,
    pub class: Class,
    pub method: &'static str,
    pub path: &'static str,
    pub status: u16,
    /// A rota tem `{project_id}` e esse projeto precisa ser o aberto.
    pub project: bool,
    pub surface: Surface,
    pub schema: Value,
}

impl OpDef {
    /// Nome da tool MCP (`runs.create` → `runs_create`).
    pub fn tool_name(&self) -> String {
        self.name.replace('.', "_")
    }
}

// ---- construtores de schema ---------------------------------------------------------------------

fn s(max: usize) -> Value {
    json!({"type": "string", "minLength": 1, "maxLength": max})
}

fn id() -> Value {
    json!({"type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Za-z0-9._:-]+$"})
}

fn int(min: i64, max: i64) -> Value {
    json!({"type": "integer", "minimum": min, "maximum": max})
}

fn boolean() -> Value {
    json!({"type": "boolean"})
}

fn limit() -> Value {
    int(1, 200)
}

fn obj(props: &[(&str, Value)], required: &[&str]) -> Value {
    let mut p = serde_json::Map::new();
    for (k, v) in props {
        p.insert((*k).to_owned(), v.clone());
    }
    json!({"type": "object", "properties": p, "required": required, "additionalProperties": false})
}

fn free_obj(max_props: usize) -> Value {
    json!({"type": "object", "maxProperties": max_props})
}

fn pid() -> (&'static str, Value) {
    ("project_id", id())
}

#[allow(clippy::too_many_arguments)]
fn op(
    name: &'static str,
    summary: &'static str,
    scope: Option<Scope>,
    mutating: bool,
    class: Class,
    method: &'static str,
    path: &'static str,
    status: u16,
    schema: Value,
) -> OpDef {
    OpDef {
        name,
        summary,
        scope,
        mutating,
        class,
        method,
        path,
        status,
        // `projects.get/open/close` endereçam o registry (não exigem o projeto aberto)
        project: path.contains("{project_id}")
            && !matches!(name, "projects.get" | "projects.open" | "projects.close"),
        surface: Surface::Both,
        schema,
    }
}

fn build() -> Vec<OpDef> {
    use Class::{Admin, Approve, Export, Read, RunStart, Upload, Write};
    use Scope::{
        AdminTokens, ExportRead, ExportStart, MediaRead, MediaWrite, ProjectRead, ProjectWrite,
        RunApprove, RunRead, RunStart as RunStartScope, WebhookManage,
    };
    let page = |extra: &[(&'static str, Value)], req: &[&str]| {
        let mut p: Vec<(&str, Value)> = vec![("after", s(128)), ("limit", limit())];
        p.extend(extra.iter().cloned());
        obj(&p, req)
    };
    let p_only = obj(&[pid()], &["project_id"]);
    let run_ref = obj(&[pid(), ("run_id", id())], &["project_id", "run_id"]);
    let mut v = vec![
        // ---- servidor ---------------------------------------------------------------------
        op(
            "server.health",
            "Liveness probe (public, minimal).",
            None,
            false,
            Read,
            "GET",
            "/v1/health",
            200,
            obj(&[], &[]),
        ),
        op(
            "server.info",
            "Server, engine and limits information.",
            Some(ProjectRead),
            false,
            Read,
            "GET",
            "/v1/server",
            200,
            obj(&[], &[]),
        ),
        // ---- tokens -----------------------------------------------------------------------
        op(
            "tokens.create",
            "Create a token (the secret is shown once).",
            Some(AdminTokens),
            true,
            Admin,
            "POST",
            "/v1/tokens",
            201,
            obj(
                &[
                    ("name", s(80)),
                    (
                        "scopes",
                        json!({"type":"array","items":s(32),"minItems":1,"maxItems":11}),
                    ),
                    ("expires_in_seconds", int(60, 31_536_000)),
                ],
                &["name", "scopes"],
            ),
        ),
        op(
            "tokens.list",
            "List tokens (never the secrets).",
            Some(AdminTokens),
            false,
            Admin,
            "GET",
            "/v1/tokens",
            200,
            obj(&[], &[]),
        ),
        op(
            "tokens.revoke",
            "Revoke a token.",
            Some(AdminTokens),
            true,
            Admin,
            "DELETE",
            "/v1/tokens/{token_id}",
            200,
            obj(&[("token_id", id())], &["token_id"]),
        ),
        op(
            "tokens.rotate",
            "Rotate a token: new secret (shown once), old one revoked.",
            Some(AdminTokens),
            true,
            Admin,
            "POST",
            "/v1/tokens/{token_id}/rotate",
            201,
            obj(&[("token_id", id())], &["token_id"]),
        ),
        op(
            "audit.list",
            "Audit log of external calls.",
            Some(AdminTokens),
            false,
            Admin,
            "GET",
            "/v1/audit",
            200,
            obj(&[("after", int(0, i64::MAX)), ("limit", int(1, 500))], &[]),
        ),
        // ---- projetos ---------------------------------------------------------------------
        op(
            "projects.create",
            "Create (and open) a project in the server's project store.",
            Some(ProjectWrite),
            true,
            Write,
            "POST",
            "/v1/projects",
            201,
            obj(&[("name", s(80))], &["name"]),
        ),
        op(
            "projects.list",
            "List the server's projects.",
            Some(ProjectRead),
            false,
            Read,
            "GET",
            "/v1/projects",
            200,
            page(&[], &[]),
        ),
        op(
            "projects.get",
            "One project.",
            Some(ProjectRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}",
            200,
            p_only.clone(),
        ),
        op(
            "projects.open",
            "Open a project (the engine hosts one at a time).",
            Some(ProjectWrite),
            true,
            Write,
            "POST",
            "/v1/projects/{project_id}/open",
            200,
            p_only.clone(),
        ),
        op(
            "projects.close",
            "Close the open project.",
            Some(ProjectWrite),
            true,
            Write,
            "POST",
            "/v1/projects/{project_id}/close",
            200,
            p_only.clone(),
        ),
        op(
            "projects.summary",
            "Revision, sequences, clips, assets and run counts.",
            Some(ProjectRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/summary",
            200,
            p_only.clone(),
        ),
        // ---- uploads + assets ----------------------------------------------------------------
        op(
            "uploads.create",
            "Stream a file into staging (raw body; Content-Length required).",
            Some(MediaWrite),
            true,
            Upload,
            "POST",
            "/v1/uploads",
            201,
            obj(
                &[
                    ("filename", s(255)),
                    (
                        "expected_sha256",
                        json!({"type":"string","pattern":"^[0-9a-f]{64}$"}),
                    ),
                ],
                &["filename"],
            ),
        ),
        op(
            "uploads.create_inline",
            "Stage a small file sent as base64 (8 MiB max).",
            Some(MediaWrite),
            true,
            Upload,
            "POST",
            "/v1/uploads/inline",
            201,
            obj(
                &[
                    ("filename", s(255)),
                    (
                        "content_base64",
                        json!({"type":"string","minLength":1,"maxLength":11_534_336}),
                    ),
                    (
                        "expected_sha256",
                        json!({"type":"string","pattern":"^[0-9a-f]{64}$"}),
                    ),
                ],
                &["filename", "content_base64"],
            ),
        ),
        op(
            "uploads.list",
            "List staged uploads.",
            Some(MediaRead),
            false,
            Read,
            "GET",
            "/v1/uploads",
            200,
            page(&[], &[]),
        ),
        op(
            "uploads.delete",
            "Delete a staged upload.",
            Some(MediaWrite),
            true,
            Write,
            "DELETE",
            "/v1/uploads/{upload_id}",
            200,
            obj(&[("upload_id", id())], &["upload_id"]),
        ),
        op(
            "assets.import",
            "Import a staged upload through the asset system (hash + probe + catalog).",
            Some(MediaWrite),
            true,
            Upload,
            "POST",
            "/v1/projects/{project_id}/assets",
            202,
            obj(&[pid(), ("upload_id", id())], &["project_id", "upload_id"]),
        ),
        op(
            "assets.list",
            "List assets.",
            Some(MediaRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/assets",
            200,
            page(&[pid()], &["project_id"]),
        ),
        op(
            "assets.get",
            "One asset.",
            Some(MediaRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/assets/{asset_id}",
            200,
            obj(&[pid(), ("asset_id", id())], &["project_id", "asset_id"]),
        ),
        op(
            "imports.get",
            "State of an asynchronous import.",
            Some(MediaRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/imports/{ticket_id}",
            200,
            obj(&[pid(), ("ticket_id", id())], &["project_id", "ticket_id"]),
        ),
        // ---- timeline -----------------------------------------------------------------------
        op(
            "sequences.list",
            "List sequences.",
            Some(ProjectRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/sequences",
            200,
            p_only.clone(),
        ),
        op(
            "sequences.get",
            "One sequence (full content; use timeline.query for large ones).",
            Some(ProjectRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/sequences/{sequence_id}",
            200,
            obj(
                &[pid(), ("sequence_id", id())],
                &["project_id", "sequence_id"],
            ),
        ),
        op(
            "timeline.query",
            "Clips of a sequence by time range, paginated, with a content digest.",
            Some(ProjectRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/sequences/{sequence_id}/timeline",
            200,
            obj(
                &[
                    pid(),
                    ("sequence_id", id()),
                    ("from_ticks", int(0, i64::MAX)),
                    ("to_ticks", int(0, i64::MAX)),
                    ("track", id()),
                    ("after", id()),
                    ("limit", int(1, 500)),
                ],
                &["project_id", "sequence_id"],
            ),
        ),
        op(
            "commands.preview",
            "Phase 1 of the write gate: validate a transaction and get a plan token (nothing is written).",
            Some(ProjectWrite),
            true,
            Write,
            "POST",
            "/v1/projects/{project_id}/commands/preview",
            200,
            obj(
                &[
                    pid(),
                    ("label", s(120)),
                    (
                        "commands",
                        json!({"type":"array","minItems":1,"maxItems":500,"items":{"type":"object"}}),
                    ),
                    ("expected_revision", int(0, i64::MAX)),
                ],
                &["project_id", "commands"],
            ),
        ),
        op(
            "commands.apply",
            "Phase 2 of the write gate: apply only the reviewed plan.",
            Some(ProjectWrite),
            true,
            Write,
            "POST",
            "/v1/projects/{project_id}/commands/apply",
            200,
            obj(
                &[pid(), ("plan_token", s(4096))],
                &["project_id", "plan_token"],
            ),
        ),
        op(
            "history.list",
            "Document history entries.",
            Some(ProjectRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/history",
            200,
            obj(
                &[pid(), ("after", int(0, i64::MAX)), ("limit", int(1, 500))],
                &["project_id"],
            ),
        ),
        // ---- AI runs ---------------------------------------------------------------------------
        op(
            "runs.create",
            "Start an AI Run from brief + raw footage + references.",
            Some(RunStartScope),
            true,
            RunStart,
            "POST",
            "/v1/projects/{project_id}/runs",
            202,
            obj(
                &[
                    pid(),
                    ("brief_text", s(60_000)),
                    (
                        "documents",
                        json!({"type":"array","items":id(),"maxItems":12}),
                    ),
                    ("assets", json!({"type":"array","items":id(),"maxItems":50})),
                    (
                        "references",
                        json!({"type":"array","items":id(),"maxItems":8}),
                    ),
                    ("note", s(2000)),
                    (
                        "deliverables",
                        json!({"type":"array","maxItems":24,"items":{"type":"object"}}),
                    ),
                    ("variants", free_obj(8)),
                    ("client_id", id()),
                    ("budget", free_obj(12)),
                    ("policy", free_obj(8)),
                    ("start", boolean()),
                ],
                &["project_id"],
            ),
        ),
        op(
            "runs.list",
            "List runs.",
            Some(RunRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/runs",
            200,
            obj(&[pid(), ("limit", int(1, 200))], &["project_id"]),
        ),
        op(
            "runs.get",
            "Run snapshot (stages, effects, provenance, pending decision).",
            Some(RunRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/runs/{run_id}",
            200,
            run_ref.clone(),
        ),
        op(
            "runs.pause",
            "Pause a run.",
            Some(RunStartScope),
            true,
            Write,
            "POST",
            "/v1/projects/{project_id}/runs/{run_id}/pause",
            200,
            run_ref.clone(),
        ),
        op(
            "runs.resume",
            "Resume a paused run.",
            Some(RunStartScope),
            true,
            RunStart,
            "POST",
            "/v1/projects/{project_id}/runs/{run_id}/resume",
            202,
            run_ref.clone(),
        ),
        op(
            "runs.cancel",
            "Cancel a run.",
            Some(RunStartScope),
            true,
            Write,
            "POST",
            "/v1/projects/{project_id}/runs/{run_id}/cancel",
            200,
            run_ref.clone(),
        ),
        op(
            "runs.approve",
            "Answer the pending approval/decision of a run.",
            Some(RunApprove),
            true,
            Approve,
            "POST",
            "/v1/projects/{project_id}/runs/{run_id}/approvals",
            200,
            obj(
                &[
                    pid(),
                    ("run_id", id()),
                    ("decision_id", id()),
                    ("option", id()),
                    ("payload", free_obj(32)),
                ],
                &["project_id", "run_id", "decision_id", "option"],
            ),
        ),
        op(
            "runs.plan",
            "Production plan, edit plans and validation of a run.",
            Some(RunRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/runs/{run_id}/plan",
            200,
            run_ref.clone(),
        ),
        op(
            "runs.review",
            "Reviews (critic) of a run.",
            Some(RunRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/runs/{run_id}/review",
            200,
            run_ref.clone(),
        ),
        op(
            "runs.cost",
            "Usage and budget of a run.",
            Some(RunRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/runs/{run_id}/cost",
            200,
            run_ref.clone(),
        ),
        op(
            "runs.events",
            "Run event log (paginated by seq).",
            Some(RunRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/runs/{run_id}/events",
            200,
            obj(
                &[pid(), ("run_id", id()), ("after", int(0, i64::MAX))],
                &["project_id", "run_id"],
            ),
        ),
        op(
            "runs.variants",
            "Create variants from a completed run.",
            Some(RunStartScope),
            true,
            RunStart,
            "POST",
            "/v1/projects/{project_id}/runs/{run_id}/variants",
            202,
            obj(
                &[
                    pid(),
                    ("run_id", id()),
                    ("count", int(1, 20)),
                    ("axis", json!({"type":"array","items":s(32),"maxItems":8})),
                ],
                &["project_id", "run_id", "count"],
            ),
        ),
        op(
            "memory.list",
            "Project/client/user memory items (read-only: promotion is a UI action).",
            Some(RunRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/memory",
            200,
            p_only.clone(),
        ),
        op(
            "gateway.status",
            "Asset Gateway and generation status.",
            Some(RunRead),
            false,
            Read,
            "GET",
            "/v1/gateway",
            200,
            obj(&[], &[]),
        ),
        // ---- exports ----------------------------------------------------------------------------
        op(
            "exports.start",
            "Queue exports of sequences (server chooses the output location).",
            Some(ExportStart),
            true,
            Export,
            "POST",
            "/v1/projects/{project_id}/exports",
            202,
            obj(
                &[
                    pid(),
                    (
                        "items",
                        json!({"type":"array","minItems":1,"maxItems":24,"items":obj(&[("sequence", id()), ("preset", json!({"enum":["h264-mp4","intermediate"]})), ("width", int(16, 8192)), ("height", int(16, 8192)), ("name", json!({"type":"string","maxLength":64})), ("encoder", id())], &["sequence"])}),
                    ),
                ],
                &["project_id", "items"],
            ),
        ),
        op(
            "exports.list",
            "List exports.",
            Some(ExportRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/exports",
            200,
            page(&[pid()], &["project_id"]),
        ),
        op(
            "exports.get",
            "Export state, report and probe.",
            Some(ExportRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/exports/{export_id}",
            200,
            obj(&[pid(), ("export_id", id())], &["project_id", "export_id"]),
        ),
        op(
            "exports.cancel",
            "Cancel the batch of an export.",
            Some(ExportStart),
            true,
            Write,
            "POST",
            "/v1/projects/{project_id}/exports/{export_id}/cancel",
            200,
            obj(&[pid(), ("export_id", id())], &["project_id", "export_id"]),
        ),
        op(
            "deliverables.list",
            "Completed exports (deliverables) of the project.",
            Some(ExportRead),
            false,
            Read,
            "GET",
            "/v1/projects/{project_id}/deliverables",
            200,
            page(&[pid()], &["project_id"]),
        ),
        // ---- webhooks ---------------------------------------------------------------------------
        op(
            "webhooks.create",
            "Register a webhook (the signing secret is shown once).",
            Some(WebhookManage),
            true,
            Write,
            "POST",
            "/v1/webhooks",
            201,
            obj(
                &[
                    ("url", s(2048)),
                    (
                        "events",
                        json!({"type":"array","items":s(40),"minItems":1,"maxItems":16}),
                    ),
                    ("description", json!({"type":"string","maxLength":200})),
                ],
                &["url", "events"],
            ),
        ),
        op(
            "webhooks.list",
            "List webhooks (never the secrets).",
            Some(WebhookManage),
            false,
            Read,
            "GET",
            "/v1/webhooks",
            200,
            obj(&[], &[]),
        ),
        op(
            "webhooks.get",
            "One webhook.",
            Some(WebhookManage),
            false,
            Read,
            "GET",
            "/v1/webhooks/{webhook_id}",
            200,
            obj(&[("webhook_id", id())], &["webhook_id"]),
        ),
        op(
            "webhooks.update",
            "Update url/events/enabled/description.",
            Some(WebhookManage),
            true,
            Write,
            "PATCH",
            "/v1/webhooks/{webhook_id}",
            200,
            obj(
                &[
                    ("webhook_id", id()),
                    ("url", s(2048)),
                    (
                        "events",
                        json!({"type":"array","items":s(40),"minItems":1,"maxItems":16}),
                    ),
                    ("enabled", boolean()),
                    ("description", json!({"type":"string","maxLength":200})),
                ],
                &["webhook_id"],
            ),
        ),
        op(
            "webhooks.rotate_secret",
            "New signing secret (shown once).",
            Some(WebhookManage),
            true,
            Write,
            "POST",
            "/v1/webhooks/{webhook_id}/rotate-secret",
            200,
            obj(&[("webhook_id", id())], &["webhook_id"]),
        ),
        op(
            "webhooks.delete",
            "Delete a webhook and its delivery log.",
            Some(WebhookManage),
            true,
            Write,
            "DELETE",
            "/v1/webhooks/{webhook_id}",
            200,
            obj(&[("webhook_id", id())], &["webhook_id"]),
        ),
        op(
            "webhooks.test",
            "Queue a synthetic `webhook.test` delivery.",
            Some(WebhookManage),
            true,
            Write,
            "POST",
            "/v1/webhooks/{webhook_id}/test",
            202,
            obj(&[("webhook_id", id())], &["webhook_id"]),
        ),
        op(
            "webhooks.deliveries",
            "Delivery log (attempt, status, latency, next retry, terminal state).",
            Some(WebhookManage),
            false,
            Read,
            "GET",
            "/v1/webhooks/{webhook_id}/deliveries",
            200,
            obj(
                &[
                    ("webhook_id", id()),
                    ("after", int(0, i64::MAX)),
                    ("limit", int(1, 200)),
                ],
                &["webhook_id"],
            ),
        ),
        op(
            "webhooks.redeliver",
            "Re-deliver one delivery (dead letters included).",
            Some(WebhookManage),
            true,
            Write,
            "POST",
            "/v1/webhooks/{webhook_id}/deliveries/{delivery_id}/redeliver",
            202,
            obj(
                &[("webhook_id", id()), ("delivery_id", int(1, i64::MAX))],
                &["webhook_id", "delivery_id"],
            ),
        ),
        // ---- eventos --------------------------------------------------------------------------------
        op(
            "events.list",
            "Server events after a cursor (polling; SSE at /v1/events/stream).",
            Some(ProjectRead),
            false,
            Read,
            "GET",
            "/v1/events",
            200,
            obj(&[("after", int(0, i64::MAX)), ("limit", int(1, 500))], &[]),
        ),
    ];
    for o in &mut v {
        if o.name == "uploads.create" {
            o.surface = Surface::RestOnly;
        }
    }
    v
}

/// O catálogo (imutável, construído uma vez).
pub fn ops() -> &'static [OpDef] {
    static OPS: OnceLock<Vec<OpDef>> = OnceLock::new();
    OPS.get_or_init(build)
}

pub fn find(name: &str) -> Option<&'static OpDef> {
    ops().iter().find(|o| o.name == name)
}

pub fn find_tool(tool: &str) -> Option<&'static OpDef> {
    ops()
        .iter()
        .find(|o| o.surface == Surface::Both && o.tool_name() == tool)
}

/// Tipos de evento publicados (assinar `*` recebe todos).
pub const EVENT_TYPES: &[&str] = &[
    "run.started",
    "run.waiting_user",
    "run.completed",
    "run.failed",
    "run.cancelled",
    "export.completed",
    "export.failed",
    "asset.imported",
    "webhook.test",
];

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_operation_declares_a_scope_except_the_public_health_probe() {
        for o in ops() {
            assert!(
                o.scope.is_some() || o.name == "server.health",
                "{} has no scope",
                o.name
            );
        }
        assert_eq!(ops().iter().filter(|o| o.scope.is_none()).count(), 1);
    }

    #[test]
    fn names_routes_and_tool_names_are_unique() {
        let mut names = BTreeSet::new();
        let mut routes = BTreeSet::new();
        let mut tools = BTreeSet::new();
        for o in ops() {
            assert!(names.insert(o.name), "duplicate name {}", o.name);
            assert!(
                routes.insert((o.method, o.path)),
                "duplicate route {} {}",
                o.method,
                o.path
            );
            assert!(tools.insert(o.tool_name()), "duplicate tool {}", o.name);
            assert!(o.path.starts_with("/v1/"), "{}", o.path);
        }
    }

    #[test]
    fn reads_never_mutate_and_mutations_are_never_get() {
        for o in ops() {
            if o.method == "GET" {
                assert!(!o.mutating, "{} is a GET but mutating", o.name);
            } else if o.name != "server.health" {
                assert!(
                    o.mutating,
                    "{} is {} but not marked mutating",
                    o.name, o.method
                );
            }
        }
    }

    #[test]
    fn every_path_parameter_is_a_required_schema_property() {
        for o in ops() {
            for seg in o.path.split('/') {
                if let Some(p) = seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                    assert!(o.schema["properties"].get(p).is_some(), "{}: {p}", o.name);
                    assert!(
                        o.schema["required"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|r| r == p),
                        "{}: {p} not required",
                        o.name
                    );
                }
            }
        }
    }

    #[test]
    fn every_schema_compiles_and_forbids_unknown_fields() {
        for o in ops() {
            jsonschema::validator_for(&o.schema).unwrap_or_else(|e| panic!("{}: {e}", o.name));
            assert_eq!(o.schema["additionalProperties"], false, "{}", o.name);
        }
    }

    #[test]
    fn nothing_dangerous_is_in_the_catalog() {
        // sem shell/filesystem/HTTP arbitrário/segredos/undo/aprovação de memória como operação
        for o in ops() {
            for bad in [
                "shell",
                "exec",
                "fs.",
                "path",
                "http",
                "credential",
                "undo",
                "memory.approve",
                "provider",
            ] {
                assert!(!o.name.contains(bad), "{} looks dangerous", o.name);
            }
            let props = o.schema["properties"].as_object().unwrap();
            for k in props.keys() {
                assert!(
                    !matches!(
                        k.as_str(),
                        "path" | "paths" | "file" | "command_line" | "api_key" | "secret"
                    ),
                    "{}: parameter `{k}` exposes the filesystem or a secret",
                    o.name
                );
            }
        }
    }
}
