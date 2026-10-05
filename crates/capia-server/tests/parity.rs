//! Paridade SEMÂNTICA entre as três portas do mesmo pipeline: `Core::call` direto, REST e MCP.
//! O mesmo roteiro roda em três diretórios de dados novos; comparamos o que importa — revisão,
//! sequences e grafo de clips, entradas de histórico (ator `api`), idempotência, escopos e códigos
//! de erro — nunca texto de resposta.
#![allow(
    unreachable_pub,
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines
)]

mod common;
#[path = "common/flow.rs"]
mod flow;
#[path = "common/receiver.rs"]
mod receiver;

use capia_server::auth::{Principal, authenticate, create_token};
use capia_server::config::ServerConfig;
use capia_server::{CallCtx, Core};
use common::*;
use flow::*;
use serde_json::{Value, json};
use std::sync::Arc;

struct CoreClient {
    core: Arc<Core>,
    principal: Principal,
}

impl Client for CoreClient {
    fn label(&self) -> &'static str {
        "direct"
    }

    fn call(&self, op: &str, args: Value) -> Result<Value, CallErr> {
        self.go(op, args, None)
    }

    fn call_idem(&self, op: &str, args: Value, key: &str) -> Result<Value, CallErr> {
        self.go(op, args, Some(key))
    }

    fn upload(&self, _: &str, _: &[u8]) -> String {
        unreachable!("uploads are not part of the parity scenario")
    }
}

impl CoreClient {
    fn go(&self, op: &str, args: Value, key: Option<&str>) -> Result<Value, CallErr> {
        let ctx = CallCtx {
            principal: Some(self.principal.clone()),
            surface: "internal",
            request_id: "req_parity".into(),
            idempotency_key: key.map(str::to_owned),
        };
        match self.core.call(&ctx, op, args) {
            Ok(r) => Ok(r.body),
            Err(e) => Err(CallErr {
                status: e.status,
                code: e.code.clone(),
                body: e.body("req_parity"),
            }),
        }
    }
}

fn outcome(step: &str, r: &Result<Value, CallErr>) -> Value {
    match r {
        Ok(_) => json!({"step": step, "ok": true}),
        Err(e) => json!({"step": step, "ok": false, "status": e.status, "code": e.code}),
    }
}

fn seq_cmds(op: &str, seq: &str) -> Value {
    create_sequence_cmds(op, seq)
}

fn track_cmd(op: &str, track: &str) -> Value {
    json!([{"operation_id": op, "type": "add_track", "sequence": "seq1", "id": track, "kind": "visual"}])
}

/// Roteiro único; devolve a transcrição semântica (sem ids aleatórios).
fn scenario(admin: &dyn Client, writer2: &dyn Client, reader: &dyn Client) -> Value {
    let mut t = Vec::new();
    let a = admin.call_idem("projects.create", json!({"name": "parity"}), "p-1");
    t.push(outcome("create", &a));
    let pid = a.unwrap()["project"]["id"].as_str().unwrap().to_owned();
    // replay: mesmo resultado, nenhum segundo projeto
    let b = admin.call_idem("projects.create", json!({"name": "parity"}), "p-1");
    t.push(json!({"step": "replay", "same_project": b.as_ref().is_ok_and(|b| b["project"]["id"] == pid.as_str())}));
    let n = admin.ok("projects.list", json!({}))["projects"]
        .as_array()
        .unwrap()
        .len();
    t.push(json!({"step": "projects_after_replay", "n": n}));
    let c = admin.call_idem("projects.create", json!({"name": "different"}), "p-1");
    t.push(outcome("key_reused", &c));
    // porta de escrita: preview → apply
    let rev0 = admin.ok("projects.summary", json!({"project_id": pid}))["revision"].clone();
    let p1 = admin.call("commands.preview", json!({"project_id": pid, "label": "make seq", "commands": seq_cmds("op-1", "seq1"), "expected_revision": rev0}));
    t.push(outcome("preview1", &p1));
    let tok1 = p1.unwrap()["plan_token"].clone();
    // outro token não aplica o plano alheio
    let steal = writer2.call(
        "commands.apply",
        json!({"project_id": pid, "plan_token": tok1}),
    );
    t.push(outcome("apply_by_other_token", &steal));
    // idempotência do apply: duas vezes com a mesma chave, um único efeito
    let ap = admin.call_idem(
        "commands.apply",
        json!({"project_id": pid, "plan_token": tok1}),
        "a-1",
    );
    t.push(outcome("apply1", &ap));
    let ap2 = admin.call_idem(
        "commands.apply",
        json!({"project_id": pid, "plan_token": tok1}),
        "a-1",
    );
    t.push(json!({"step": "apply1_replay", "same": ap2.is_ok() && ap2.as_ref().unwrap() == ap.as_ref().unwrap()}));
    // revisão velha
    let stale = admin.call(
        "commands.preview",
        json!({"project_id": pid, "commands": seq_cmds("op-2", "seq2"), "expected_revision": rev0}),
    );
    t.push(outcome("stale_revision", &stale));
    // plano velho (documento mudou entre preview e apply)
    let pa = admin.ok(
        "commands.preview",
        json!({"project_id": pid, "commands": track_cmd("op-3", "t1")}),
    );
    let pb = admin.ok(
        "commands.preview",
        json!({"project_id": pid, "commands": track_cmd("op-4", "t1")}),
    );
    let okb = admin.call(
        "commands.apply",
        json!({"project_id": pid, "plan_token": pb["plan_token"]}),
    );
    t.push(outcome("apply_b", &okb));
    let drift = admin.call(
        "commands.apply",
        json!({"project_id": pid, "plan_token": pa["plan_token"]}),
    );
    t.push(outcome("apply_a_after_drift", &drift));
    // comando inválido: recusado antes de gravar
    let bad = admin.call("commands.preview", json!({"project_id": pid, "commands": [{"operation_id": "op-5", "type": "add_track", "sequence": "nope", "id": "t9", "kind": "visual"}]}));
    t.push(outcome("invalid_command", &bad));
    // escopos
    for (step, op, args) in [
        ("reader_create", "projects.create", json!({"name": "x"})),
        (
            "reader_preview",
            "commands.preview",
            json!({"project_id": pid, "commands": seq_cmds("op-9", "s9")}),
        ),
        ("reader_webhooks", "webhooks.list", json!({})),
        ("reader_tokens", "tokens.list", json!({})),
        ("reader_runs", "runs.list", json!({"project_id": pid})),
    ] {
        t.push(outcome(step, &reader.call(op, args)));
    }
    t.push(outcome(
        "reader_read",
        &reader.call("sequences.list", json!({"project_id": pid})),
    ));
    // erros de pedido
    for (step, op, args) in [
        ("create_no_name", "projects.create", json!({})),
        (
            "create_extra",
            "projects.create",
            json!({"name": "x", "bogus": 1}),
        ),
        // (ids com `/`/`..` não existem na REST: o caminho nem casa a rota; o MCP/Core recusam por
        // schema — coberto em mcp.rs/rest_core.rs, fora da transcrição comparável)
        (
            "get_unknown",
            "projects.get",
            json!({"project_id": "prj_nope"}),
        ),
        (
            "seq_unknown",
            "sequences.get",
            json!({"project_id": pid, "sequence_id": "nope"}),
        ),
        (
            "run_unknown",
            "runs.get",
            json!({"project_id": pid, "run_id": "nope"}),
        ),
        ("events_limit", "events.list", json!({"limit": 100_000})),
    ] {
        t.push(outcome(step, &admin.call(op, args)));
    }
    // estado final
    let summary = admin.ok("projects.summary", json!({"project_id": pid}));
    let list = admin.ok("sequences.list", json!({"project_id": pid}));
    let seq = admin.ok(
        "sequences.get",
        json!({"project_id": pid, "sequence_id": "seq1"}),
    );
    let hist = admin.ok("history.list", json!({"project_id": pid}));
    let entries: Vec<Value> = hist["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            json!({
                "label": e["label"], "actor_kind": e["actor"]["kind"],
                "token_actor": e["actor"]["id"].as_str().is_some_and(|i| i.starts_with("token:tok_")),
                "ops": e["operation_ids"].clone().as_array().map_or(0, Vec::len),
            })
        })
        .collect();
    t.push(json!({
        "step": "final_state",
        "revision": summary["revision"], "sequences": summary["sequences"], "clips": summary["clips"],
        "listed": list["sequences"].as_array().unwrap().iter().map(|s| json!([s["id"], s["name"], s["width"], s["height"], s["frame_rate"], s["clip_count"]])).collect::<Vec<_>>(),
        "list_revision": list["revision"],
        "shape": sequence_shape(&seq, "RUN"),
        "tracks": seq["sequence"]["tracks"],
        "history": entries,
        "can_undo": summary["can_undo"],
    }));
    Value::Array(t)
}

fn audit_counts(core: &Core) -> Value {
    let rows = core.db.audit_list(0, 5000).unwrap();
    let count = |f: &dyn Fn(&capia_store::AuditRow) -> bool| rows.iter().filter(|r| f(r)).count();
    json!({
        "replayed": count(&|r| r.outcome == "replayed"),
        "scope_denied": count(&|r| r.code.as_deref() == Some("INSUFFICIENT_SCOPE")),
        "stale_revision": count(&|r| r.code.as_deref() == Some("REVISION_CONFLICT")),
        "key_reused": count(&|r| r.code.as_deref() == Some("IDEMPOTENCY_KEY_REUSED")),
    })
}

fn run_http(mcp: bool) -> (Value, Value) {
    let s = start(if mcp { "parity-mcp" } else { "parity-rest" }, |_| {});
    let w2 = s.token("writer2", &["project:write", "project:read"]);
    let rd = s.token("reader", &["project:read"]);
    let tr = s.admin.clone();
    let (t, audit);
    if mcp {
        let (a, b, c) = (
            McpClient { s: &s, token: tr },
            McpClient { s: &s, token: w2 },
            McpClient { s: &s, token: rd },
        );
        t = scenario(&a, &b, &c);
    } else {
        let (a, b, c) = (
            RestClient { s: &s, token: tr },
            RestClient { s: &s, token: w2 },
            RestClient { s: &s, token: rd },
        );
        t = scenario(&a, &b, &c);
    }
    audit = audit_counts(s.core());
    // a superfície fica registrada na auditoria
    let surf = if mcp { "mcp" } else { "rest" };
    assert!(
        s.core()
            .db
            .audit_list(0, 5000)
            .unwrap()
            .iter()
            .any(|r| r.surface == surf && r.op == "commands.apply")
    );
    (t, audit)
}

fn run_direct() -> (Value, Value) {
    let dir = TempDir::new("parity-direct");
    let mut cfg = ServerConfig::new(dir.path());
    cfg.secrets = Arc::new(capia_secrets::MemoryStore::new());
    let core = Core::open(cfg).unwrap();
    let mk = |name: &str, scopes: &[&str]| {
        let sc: Vec<String> = scopes.iter().map(|s| (*s).to_owned()).collect();
        let (_, secret) = create_token(&core.db, name, &sc, None).unwrap();
        CoreClient {
            core: Arc::clone(&core),
            principal: authenticate(&core.db, &secret).unwrap(),
        }
    };
    let all: Vec<&str> = capia_server::scope::ALL_SCOPES
        .iter()
        .map(|s| s.as_str())
        .collect();
    let a = mk("admin", &all);
    let b = mk("writer2", &["project:write", "project:read"]);
    let c = mk("reader", &["project:read"]);
    let t = scenario(&a, &b, &c);
    let audit = audit_counts(&core);
    let _ = core.session_call("project.close", json!({}));
    drop(dir);
    (t, audit)
}

#[test]
fn direct_core_rest_and_mcp_have_identical_semantics() {
    let (direct, d_audit) = run_direct();
    let (rest, r_audit) = run_http(false);
    let (mcp, m_audit) = run_http(true);
    // o roteiro realmente exercitou o que promete
    let steps: Vec<&str> = direct
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["step"].as_str().unwrap())
        .collect();
    for want in [
        "replay",
        "key_reused",
        "stale_revision",
        "apply_a_after_drift",
        "reader_create",
        "final_state",
    ] {
        assert!(steps.contains(&want), "{want}");
    }
    let find = |t: &Value, step: &str| {
        t.as_array()
            .unwrap()
            .iter()
            .find(|s| s["step"] == step)
            .unwrap()
            .clone()
    };
    assert_eq!(find(&direct, "replay")["same_project"], true);
    assert_eq!(find(&direct, "projects_after_replay")["n"], 1);
    assert_eq!(
        find(&direct, "key_reused")["code"],
        "IDEMPOTENCY_KEY_REUSED"
    );
    assert_eq!(find(&direct, "stale_revision")["code"], "REVISION_CONFLICT");
    assert_eq!(
        find(&direct, "apply_a_after_drift")["code"],
        "PLAN_STATE_CHANGED"
    );
    assert_eq!(find(&direct, "reader_create")["code"], "INSUFFICIENT_SCOPE");
    assert_eq!(find(&direct, "apply_by_other_token")["status"], 403);
    assert_eq!(find(&direct, "apply1_replay")["same"], true);
    let fin = find(&direct, "final_state");
    assert_eq!(fin["sequences"], 1);
    assert!(
        fin["history"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["actor_kind"] == "api" && e["token_actor"] == true),
        "{fin}"
    );
    assert!(!fin["history"].as_array().unwrap().is_empty());
    // as três portas dizem o mesmo
    assert_eq!(direct, rest, "direct Core::call vs REST");
    assert_eq!(direct, mcp, "direct Core::call vs MCP");
    assert_eq!(d_audit, r_audit);
    assert_eq!(d_audit, m_audit);
    assert_eq!(d_audit["replayed"], 2, "{d_audit}");
    assert!(d_audit["scope_denied"].as_u64().unwrap() >= 5);
}
