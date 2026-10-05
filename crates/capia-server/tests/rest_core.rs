//! REST `/v1`: autenticação, scopes, tokens, idempotência, gate de escrita, limites e cabeçalhos.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use serde_json::json;

#[test]
fn health_is_public_and_everything_else_needs_a_token() {
    let s = start("health", |_| {});
    let h = request(s.addr, "GET", "/v1/health", None, &[], None);
    assert_eq!(h.status, 200);
    assert_eq!(h.json()["status"], "ok");
    assert!(h.header("x-content-type-options").is_some());
    for path in [
        "/v1/server",
        "/v1/projects",
        "/v1/tokens",
        "/v1/webhooks",
        "/v1/events",
    ] {
        let r = request(s.addr, "GET", path, None, &[], None);
        assert_eq!(r.status, 401, "{path}");
        let b = r.json();
        assert_eq!(b["code"], "UNAUTHORIZED");
        assert!(b["request_id"].as_str().unwrap().starts_with("req_"), "{b}");
        assert_eq!(r.header("x-request-id"), b["request_id"].as_str());
        assert!(r.header("www-authenticate").unwrap().starts_with("Bearer"));
    }
    // token malformado/inventado: mesma resposta
    for t in [
        "",
        "x",
        "capia_00",
        &format!("capia_{}", "0".repeat(64)),
        &"a".repeat(500),
    ] {
        let r = request(s.addr, "GET", "/v1/server", Some(t), &[], None);
        assert_eq!(r.status, 401, "token `{t}`");
    }
    assert_eq!(
        request(s.addr, "GET", "/v1/nope", Some(&s.admin), &[], None).status,
        404
    );
    let m = request(s.addr, "PUT", "/v1/projects", Some(&s.admin), &[], None);
    assert_eq!(m.status, 405);
    assert!(m.header("allow").unwrap().contains("GET"));
}

#[test]
fn scopes_are_enforced_per_operation_with_a_structured_403() {
    let s = start("scopes", |_| {});
    let reader = s.token("reader", &["project:read"]);
    assert_eq!(s.call_as(&reader, "GET", "/v1/server", None).status, 200);
    let denied = s.call_as(&reader, "POST", "/v1/projects", Some(json!({"name": "x"})));
    assert_eq!(denied.status, 403);
    assert_eq!(denied.code(), "INSUFFICIENT_SCOPE");
    assert_eq!(denied.json()["details"]["required_scope"], "project:write");
    // outras famílias
    assert_eq!(s.call_as(&reader, "GET", "/v1/tokens", None).status, 403);
    assert_eq!(s.call_as(&reader, "GET", "/v1/webhooks", None).status, 403);
    assert_eq!(s.call_as(&reader, "GET", "/v1/audit", None).status, 403);
    let pid = s.create_project("p");
    assert_eq!(
        s.call_as(&reader, "GET", &format!("/v1/projects/{pid}/runs"), None)
            .status,
        403
    );
    assert_eq!(
        s.call_as(&reader, "GET", &format!("/v1/projects/{pid}/exports"), None)
            .status,
        403
    );
    assert_eq!(
        s.call_as(&reader, "GET", &format!("/v1/projects/{pid}/assets"), None)
            .status,
        403
    );
    assert_eq!(
        s.call_as(&reader, "GET", &format!("/v1/projects/{pid}/summary"), None)
            .status,
        200
    );
}

#[test]
fn token_lifecycle_secret_once_revoke_rotate_and_no_escalation() {
    let s = start("tokens", |_| {});
    let created = s.call(
        "POST",
        "/v1/tokens",
        Some(json!({"name": "ci", "scopes": ["project:read", "run:read"]})),
    );
    assert_eq!(created.status, 201);
    let body = created.json();
    let secret = body["secret"].as_str().unwrap().to_owned();
    assert!(secret.starts_with("capia_") && secret.len() == 70);
    let id = body["token"]["id"].as_str().unwrap().to_owned();
    // a listagem nunca mostra segredo nem hash
    let list = s.call("GET", "/v1/tokens", None);
    let text = String::from_utf8_lossy(&list.body).into_owned();
    assert!(
        !text.contains(&secret) && !text.contains("secret_hash"),
        "{text}"
    );
    assert_eq!(s.call_as(&secret, "GET", "/v1/server", None).status, 200);
    // o segredo não está em lugar nenhum do disco (só o SHA-256)
    for entry in walk(s.dir.path()) {
        if let Ok(bytes) = std::fs::read(&entry) {
            assert!(
                !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
                "the token secret leaked into {}",
                entry.display()
            );
        }
    }
    // sem escalada: um token só concede scopes que possui
    let limited = s.token("limited", &["admin:tokens", "project:read"]);
    let esc = s.call_as(
        &limited,
        "POST",
        "/v1/tokens",
        Some(json!({"name": "x", "scopes": ["project:write"]})),
    );
    assert_eq!(esc.status, 403);
    assert_eq!(esc.code(), "SCOPE_ESCALATION");
    let ok = s.call_as(
        &limited,
        "POST",
        "/v1/tokens",
        Some(json!({"name": "x", "scopes": ["project:read"]})),
    );
    assert_eq!(ok.status, 201);
    // rotacionar um token mais forte que o chamador também é escalada
    let strong = s
        .call(
            "POST",
            "/v1/tokens",
            Some(json!({"name": "strong", "scopes": ["project:write", "media:write"]})),
        )
        .json();
    let esc2 = s.call_as(
        &limited,
        "POST",
        &format!(
            "/v1/tokens/{}/rotate",
            strong["token"]["id"].as_str().unwrap()
        ),
        None,
    );
    assert_eq!(esc2.status, 403);
    // rotação: novo segredo funciona, o antigo morre
    let rot = s.call("POST", &format!("/v1/tokens/{id}/rotate"), None);
    assert_eq!(rot.status, 201);
    let new_secret = rot.json()["secret"].as_str().unwrap().to_owned();
    assert_eq!(s.call_as(&secret, "GET", "/v1/server", None).status, 401);
    assert_eq!(
        s.call_as(&new_secret, "GET", "/v1/server", None).status,
        200
    );
    // revogação
    let new_id = rot.json()["token"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        s.call("DELETE", &format!("/v1/tokens/{new_id}"), None)
            .json()["revoked"],
        true
    );
    assert_eq!(
        s.call_as(&new_secret, "GET", "/v1/server", None).status,
        401
    );
    assert_eq!(s.call("DELETE", "/v1/tokens/tok_nope", None).status, 404);
    // expiração
    let short = s
        .call(
            "POST",
            "/v1/tokens",
            Some(json!({"name": "short", "scopes": ["project:read"], "expires_in_seconds": 60})),
        )
        .json();
    assert!(short["token"]["expires_ms"].as_u64().is_some());
    // scopes inválidos/duplicados/vazios
    for bad in [
        json!([]),
        json!(["nope:read"]),
        json!(["project:read", "project:read"]),
    ] {
        let r = s.call(
            "POST",
            "/v1/tokens",
            Some(json!({"name": "b", "scopes": bad})),
        );
        assert!(r.status == 422, "{bad}: {}", r.status);
    }
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn idempotency_keys_replay_reject_mismatch_and_never_store_secrets() {
    let s = start("idem", |_| {});
    let a = s.call_idem("key-1", "POST", "/v1/projects", json!({"name": "A"}));
    assert_eq!(a.status, 201);
    assert!(a.header("idempotent-replay").is_none());
    let again = s.call_idem("key-1", "POST", "/v1/projects", json!({"name": "A"}));
    assert_eq!(again.status, 201);
    assert_eq!(again.header("idempotent-replay"), Some("true"));
    assert_eq!(again.json()["project"]["id"], a.json()["project"]["id"]);
    // mesma chave, outro pedido
    let other = s.call_idem("key-1", "POST", "/v1/projects", json!({"name": "B"}));
    assert_eq!(other.status, 422);
    assert_eq!(other.code(), "IDEMPOTENCY_KEY_REUSED");
    // só um projeto foi criado
    assert_eq!(
        s.call("GET", "/v1/projects", None).json()["projects"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // sem chave, cada chamada cria um projeto novo
    s.create_project("C");
    assert_eq!(
        s.call("GET", "/v1/projects", None).json()["projects"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // segredo único: o replay devolve a resposta SEM o segredo
    let t1 = s.call_idem(
        "key-tok",
        "POST",
        "/v1/tokens",
        json!({"name": "t", "scopes": ["project:read"]}),
    );
    assert!(t1.json()["secret"].is_string());
    let t2 = s.call_idem(
        "key-tok",
        "POST",
        "/v1/tokens",
        json!({"name": "t", "scopes": ["project:read"]}),
    );
    assert_eq!(t2.header("idempotent-replay"), Some("true"));
    assert!(t2.json().get("secret").is_none());
    assert_eq!(t2.json()["secret_unavailable_on_replay"], true);
    assert_eq!(t2.json()["token"]["id"], t1.json()["token"]["id"]);
    // chave inválida
    let bad = request(
        s.addr,
        "POST",
        "/v1/projects",
        Some(&s.admin),
        &[
            ("Content-Type", "application/json"),
            ("Idempotency-Key", &"k".repeat(200)),
        ],
        Some(b"{\"name\":\"x\"}"),
    );
    assert_eq!(bad.status, 400);
}

#[test]
fn writes_go_through_preview_then_apply_with_an_api_actor_per_token() {
    let s = start("gate", |_| {});
    let pid = s.create_project("gate");
    let base = format!("/v1/projects/{pid}");
    let pv = s.call(
        "POST",
        &format!("{base}/commands/preview"),
        Some(json!({"commands": create_sequence_cmds("op-1", "seq1"), "expected_revision": 0})),
    );
    assert_eq!(pv.status, 200, "{}", String::from_utf8_lossy(&pv.body));
    let token = pv.json()["plan_token"].as_str().unwrap().to_owned();
    // nada foi gravado pelo preview
    assert_eq!(
        s.call("GET", &format!("{base}/sequences"), None).json()["sequences"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    // outro token não consegue aplicar o plano de quem fez o preview
    let other = s.token("other", &["project:write"]);
    let steal = s.call_as(
        &other,
        "POST",
        &format!("{base}/commands/apply"),
        Some(json!({"plan_token": token})),
    );
    assert_eq!(
        steal.status,
        403,
        "{}",
        String::from_utf8_lossy(&steal.body)
    );
    let ap = s.call(
        "POST",
        &format!("{base}/commands/apply"),
        Some(json!({"plan_token": token})),
    );
    assert_eq!(ap.status, 200, "{}", String::from_utf8_lossy(&ap.body));
    assert_eq!(
        s.call("GET", &format!("{base}/sequences"), None).json()["sequences"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // a história registra o ator `api`
    let h = s.call("GET", &format!("{base}/history"), None).json();
    let actors: Vec<String> = h["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| format!("{}", e["actor"]))
        .collect();
    assert!(
        actors
            .iter()
            .any(|a| a.contains("api") && a.contains("token:tok_")),
        "{actors:?}"
    );
    // concorrência otimista: revisão velha ⇒ 409
    let stale = s.call(
        "POST",
        &format!("{base}/commands/preview"),
        Some(json!({"commands": create_sequence_cmds("op-2", "seq2"), "expected_revision": 0})),
    );
    assert_eq!(stale.status, 409);
    assert_eq!(stale.code(), "REVISION_CONFLICT");
    assert_eq!(stale.json()["details"]["expected_revision"], 0);
    // plano velho: o documento mudou depois do preview ⇒ 409 PLAN_STATE_CHANGED
    let rev = s.call("GET", &format!("{base}/summary"), None).json()["revision"]
        .as_u64()
        .unwrap();
    let p1 = s.call("POST", &format!("{base}/commands/preview"), Some(json!({"commands": json!([{"operation_id":"op-3","type":"add_track","sequence":"seq1","id":"t1","kind":"visual"}]), "expected_revision": rev}))).json();
    let p2 = s.call("POST", &format!("{base}/commands/preview"), Some(json!({"commands": json!([{"operation_id":"op-4","type":"add_track","sequence":"seq1","id":"t1","kind":"visual"}])}))).json();
    assert_eq!(
        s.call(
            "POST",
            &format!("{base}/commands/apply"),
            Some(json!({"plan_token": p2["plan_token"]}))
        )
        .status,
        200
    );
    let drift = s.call(
        "POST",
        &format!("{base}/commands/apply"),
        Some(json!({"plan_token": p1["plan_token"]})),
    );
    assert_eq!(
        drift.status,
        409,
        "{}",
        String::from_utf8_lossy(&drift.body)
    );
    assert_eq!(drift.code(), "PLAN_STATE_CHANGED");
    // comando inválido: recusado no preview, nunca gravado
    let bad = s.call("POST", &format!("{base}/commands/preview"), Some(json!({"commands": json!([{"operation_id":"op-5","type":"add_track","sequence":"nope","id":"t9","kind":"visual"}])})));
    assert!(bad.status == 422 || bad.status == 404, "{}", bad.status);
    // timeline.query
    let q = s.call(
        "GET",
        &format!("{base}/sequences/seq1/timeline?limit=10"),
        None,
    );
    assert_eq!(q.status, 200, "{}", String::from_utf8_lossy(&q.body));
    assert_eq!(q.json()["total"], 0);
    assert_eq!(q.json()["digest"].as_str().unwrap().len(), 64);
}

#[test]
fn only_the_open_project_accepts_project_routes() {
    let s = start("open", |_| {});
    let a = s.create_project("A");
    let b = s.create_project("B");
    // criar B abriu B; A precisa ser aberto antes de qualquer rota dele
    let r = s.call("GET", &format!("/v1/projects/{a}/summary"), None);
    assert_eq!(r.status, 409);
    assert_eq!(r.code(), "PROJECT_NOT_OPEN");
    assert_eq!(
        s.call("GET", "/v1/projects/prj_nope/summary", None).status,
        404
    );
    assert_eq!(
        s.call("POST", &format!("/v1/projects/{a}/open"), None)
            .status,
        200
    );
    assert_eq!(
        s.call("GET", &format!("/v1/projects/{a}/summary"), None)
            .status,
        200
    );
    assert_eq!(
        s.call("GET", &format!("/v1/projects/{b}/summary"), None)
            .code(),
        "PROJECT_NOT_OPEN"
    );
    let list = s.call("GET", "/v1/projects", None).json();
    let open: Vec<_> = list["projects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["open"] == true)
        .collect();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0]["id"], a.as_str());
}

#[test]
fn host_cors_and_request_shape_are_locked_down() {
    let s = start("shape", |c| {
        c.cors_origins = vec!["http://localhost:3000".into()];
        c.max_json_bytes = 4096;
    });
    // Host de outro nome (DNS rebinding) ⇒ 421
    let evil = request(
        s.addr,
        "GET",
        "/v1/health",
        None,
        &[("Host", "evil.example:80")],
        None,
    );
    let evil = {
        // `request` já injeta o Host correto; monta cru para trocar
        let _ = evil;
        parse(&raw(
            s.addr,
            b"GET /v1/health HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n",
        ))
    };
    assert_eq!(evil.status, 421);
    assert_eq!(
        parse(&raw(
            s.addr,
            b"GET /v1/health HTTP/1.1\r\nConnection: close\r\n\r\n"
        ))
        .status,
        421
    );
    // CORS: origem não listada ⇒ 403; listada ⇒ cabeçalhos; `*` nunca
    let no = request(
        s.addr,
        "GET",
        "/v1/health",
        None,
        &[("Origin", "https://evil.example")],
        None,
    );
    assert_eq!(no.status, 403);
    assert_eq!(no.code(), "CORS_DENIED");
    let yes = request(
        s.addr,
        "GET",
        "/v1/health",
        None,
        &[("Origin", "http://localhost:3000")],
        None,
    );
    assert_eq!(yes.status, 200);
    assert_eq!(
        yes.header("access-control-allow-origin"),
        Some("http://localhost:3000")
    );
    let pre = request(
        s.addr,
        "OPTIONS",
        "/v1/projects",
        None,
        &[
            ("Origin", "http://localhost:3000"),
            ("Access-Control-Request-Method", "POST"),
        ],
        None,
    );
    assert_eq!(pre.status, 204);
    assert!(
        pre.header("access-control-allow-headers")
            .unwrap()
            .contains("idempotency-key")
    );
    let pre_bad = request(
        s.addr,
        "OPTIONS",
        "/v1/projects",
        None,
        &[("Origin", "https://evil.example")],
        None,
    );
    assert_eq!(pre_bad.status, 403);
    // corpo grande demais
    let big = vec![b' '; 5000];
    let r = request(
        s.addr,
        "POST",
        "/v1/projects",
        Some(&s.admin),
        &[("Content-Type", "application/json")],
        Some(&big),
    );
    assert_eq!(r.status, 413);
    // aninhamento
    let deep = format!("{}{}", "[".repeat(64), "]".repeat(64));
    let r = s.call("POST", "/v1/projects", None);
    assert_eq!(r.status, 422, "empty body fails the schema, not the server");
    let r = request(
        s.addr,
        "POST",
        "/v1/projects",
        Some(&s.admin),
        &[("Content-Type", "application/json")],
        Some(deep.as_bytes()),
    );
    assert_eq!(r.status, 400);
    // JSON quebrado, corpo não-objeto, tipo de conteúdo errado
    let r = request(
        s.addr,
        "POST",
        "/v1/projects",
        Some(&s.admin),
        &[("Content-Type", "application/json")],
        Some(b"{nope"),
    );
    assert_eq!((r.status, r.code().as_str()), (400, "BAD_JSON"));
    let r = request(
        s.addr,
        "POST",
        "/v1/projects",
        Some(&s.admin),
        &[("Content-Type", "application/json")],
        Some(b"[1]"),
    );
    assert_eq!(r.status, 400);
    let r = request(
        s.addr,
        "POST",
        "/v1/projects",
        Some(&s.admin),
        &[("Content-Type", "text/plain")],
        Some(b"{\"name\":\"x\"}"),
    );
    assert_eq!(r.status, 415);
    // campos desconhecidos são rejeitados (additionalProperties: false)
    let r = s.call(
        "POST",
        "/v1/projects",
        Some(json!({"name": "x", "path": "/etc"})),
    );
    assert_eq!(r.status, 422);
    // smuggling/cabeçalhos malformados
    let te = parse(&raw(s.addr, format!("POST /v1/projects HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n0\r\n\r\n", s.addr.port()).as_bytes()));
    assert_eq!(te.status, 501);
    let dup = parse(&raw(s.addr, format!("POST /v1/projects HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 1\r\nContent-Length: 2\r\nConnection: close\r\n\r\nab", s.addr.port()).as_bytes()));
    assert_eq!(dup.status, 400);
    let junk = raw(s.addr, b"\x00\x01garbage\r\n\r\n");
    assert!(junk.is_empty() || parse(&junk).status == 400);
    let huge_line = format!("GET /{} HTTP/1.1\r\nHost: x\r\n\r\n", "a".repeat(20_000));
    let r = parse(&raw(s.addr, huge_line.as_bytes()));
    assert!(r.status == 431 || r.status == 400, "{}", r.status);
    // o corpo de erro tem o envelope completo
    let e = request(s.addr, "GET", "/v1/nope", Some(&s.admin), &[], None).json();
    for k in ["code", "message", "request_id"] {
        assert!(e.get(k).is_some(), "{e}");
    }
}

#[test]
fn rate_limits_answer_429_with_retry_after() {
    let s = start("rate", |c| c.rate_scale = 0.1);
    let t = s.token("burst", &["project:read"]);
    let mut limited = None;
    for _ in 0..80 {
        let r = s.call_as(&t, "GET", "/v1/server", None);
        if r.status == 429 {
            limited = Some(r);
            break;
        }
    }
    let r = limited.expect("a 429 must appear");
    assert_eq!(r.code(), "RATE_LIMITED");
    assert!(r.header("retry-after").unwrap().parse::<u64>().unwrap() >= 1);
    // outro token não é afetado
    assert_eq!(s.call("GET", "/v1/server", None).status, 200);
}

#[test]
fn the_openapi_document_is_served_and_matches_the_catalog() {
    let s = start("openapi", |_| {});
    let r = request(s.addr, "GET", "/v1/openapi.json", None, &[], None);
    assert_eq!(r.status, 200);
    let doc = r.json();
    assert_eq!(doc["openapi"], "3.1.0");
    assert_eq!(
        doc["paths"]["/v1/projects"]["post"]["x-capia-scope"],
        "project:write"
    );
    assert_eq!(doc["paths"]["/v1/health"]["get"]["security"], json!([]));
}

#[test]
fn the_audit_log_records_external_writes_with_token_and_revisions() {
    let s = start("audit", |_| {});
    let pid = s.create_project("aud");
    let pv = s
        .call(
            "POST",
            &format!("/v1/projects/{pid}/commands/preview"),
            Some(json!({"commands": create_sequence_cmds("a1", "sa")})),
        )
        .json();
    s.call(
        "POST",
        &format!("/v1/projects/{pid}/commands/apply"),
        Some(json!({"plan_token": pv["plan_token"]})),
    );
    let denied = s.token("ro", &["project:read"]);
    s.call_as(
        &denied,
        "POST",
        "/v1/projects",
        Some(json!({"name": "nope"})),
    );
    let a = s.call("GET", "/v1/audit", None).json();
    let entries = a["entries"].as_array().unwrap();
    let apply = entries
        .iter()
        .find(|e| e["op"] == "commands.apply")
        .unwrap();
    assert_eq!(apply["outcome"], "ok");
    assert_eq!(apply["surface"], "rest");
    assert!(apply["revision_before"].as_u64().unwrap() < apply["revision_after"].as_u64().unwrap());
    assert!(apply["token_id"].as_str().unwrap().starts_with("tok_"));
    let den = entries
        .iter()
        .find(|e| e["op"] == "projects.create" && e["outcome"] == "error")
        .unwrap();
    assert_eq!(den["code"], "INSUFFICIENT_SCOPE");
}

#[test]
fn after_shutdown_starts_writes_are_refused_with_503() {
    let s = start("shutdown", |_| {});
    let core = s.core().clone();
    core.shutting_down
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let r = s.call("POST", "/v1/projects", Some(json!({"name": "late"})));
    assert_eq!(r.status, 503);
    assert_eq!(r.code(), "SHUTTING_DOWN");
    // leituras continuam
    assert_eq!(s.call("GET", "/v1/server", None).status, 200);
    assert_eq!(
        request(s.addr, "GET", "/v1/health", None, &[], None).json()["status"],
        "shutting_down"
    );
    core.shutting_down
        .store(false, std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn a_full_queue_answers_503_instead_of_spawning_unbounded_work() {
    let s = start("backpressure", |c| {
        c.workers = 1;
        c.queue = 1;
    });
    // ocupa o único worker com uma conexão que nunca termina o cabeçalho
    let mut hold = std::net::TcpStream::connect(s.addr).unwrap();
    std::io::Write::write_all(&mut hold, b"GET /v1/health HTTP/1.1\r\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let mut queued = std::net::TcpStream::connect(s.addr).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    // a fila (1) está ocupada pela conexão acima; a próxima leva 503 imediato
    let mut saw_503 = false;
    for _ in 0..5 {
        let out = raw(s.addr, b"GET /v1/health HTTP/1.1\r\nHost: x\r\n\r\n");
        if !out.is_empty() && parse(&out).status == 503 {
            saw_503 = true;
            break;
        }
    }
    assert!(saw_503, "expected a 503 OVERLOADED");
    drop(hold);
    drop(queued.try_clone());
    let _ = std::io::Write::write_all(&mut queued, b"\r\n");
}
