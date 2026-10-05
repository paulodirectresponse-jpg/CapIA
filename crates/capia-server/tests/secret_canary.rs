//! Canário de segredo (Track D-2): um valor conhecido do processo (registrado no redator central) é
//! empurrado por TODO caminho que ecoa entrada — nomes, rótulos, payloads, webhooks, nomes de upload,
//! brief, chaves de idempotência, ids de requisição, caminhos, query, Origin/Host, erros — e nunca
//! pode aparecer em corpo/cabeçalho de resposta, no `server.db` (+WAL), em arquivo do data dir, no
//! stream SSE, no OpenAPI, no `server.json` nem no stdout/stderr de um `capia-server serve` real.
//! A exceção documentada: o `--bootstrap` imprime o token UMA vez no stdout.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "common/attack.rs"]
mod attack;
mod common;

use attack::*;
use common::*;
use serde_json::{Value, json};

const CANARY: &str = "CNRY-sk-live-9f2c61a07b3d48e5a1c0d77e";

fn all_text(rs: &[Resp]) -> Vec<String> {
    rs.iter().map(whole).collect()
}

fn assert_clean(label: &str, texts: &[String]) {
    for t in texts {
        assert!(!t.contains(CANARY), "{label}: the canary leaked:\n{t}");
    }
}

#[test]
fn the_canary_never_comes_back_through_any_input_path() {
    capia_secrets::register_global(CANARY);
    let s = start("canary", |c| c.rate_scale = 100.0);
    let mut seen: Vec<Resp> = Vec::new();
    let mut push = |r: Option<Resp>| {
        if let Some(r) = r {
            seen.push(r);
        }
    };
    let json_h = [("Content-Type", "application/json")];
    let call = |m: &str, p: &str, b: Value| {
        send(&s, m, p, Some(&s.admin), &json_h, b.to_string().as_bytes())
    };

    // projeto, comandos com rótulo/payload
    push(call(
        "POST",
        "/v1/projects",
        json!({"name": format!("proj {CANARY}")}),
    ));
    let pid = s.core().db.project_list(None, 10).unwrap()[0].id.clone();
    let prev = call(
        "POST",
        &format!("/v1/projects/{pid}/commands/preview"),
        json!({"label": CANARY, "commands": [{"operation_id": "c1", "type": "create_sequence", "id": "s1",
            "name": CANARY, "frame_rate": "30", "width": 1080, "height": 1920}]}),
    )
    .unwrap();
    let token = prev.json()["plan_token"].clone();
    push(Some(prev));
    push(call(
        "POST",
        &format!("/v1/projects/{pid}/commands/apply"),
        json!({"plan_token": token}),
    ));
    push(call(
        "POST",
        &format!("/v1/projects/{pid}/commands/preview"),
        json!({"commands": [{"operation_id": CANARY, "type": CANARY, "id": CANARY}]}),
    ));
    push(call(
        "GET",
        &format!("/v1/projects/{pid}/sequences"),
        json!({}),
    ));
    push(call(
        "GET",
        &format!("/v1/projects/{pid}/history"),
        json!({}),
    ));
    // webhooks: descrição e URL com o canário
    push(call(
        "POST",
        "/v1/webhooks",
        json!({"url": format!("https://example.com/hook?token={CANARY}"), "events": ["*"], "description": CANARY}),
    ));
    push(call(
        "POST",
        "/v1/webhooks",
        json!({"url": format!("https://{CANARY}.internal/x"), "events": ["*"]}),
    ));
    push(call("GET", "/v1/webhooks", json!({})));
    // uploads: nome (header, inline) e conteúdo de nome
    push(upload(
        &s,
        &s.admin,
        &pct(&format!("{CANARY}.png")),
        &tiny_png(),
        &[],
    ));
    push(call(
        "POST",
        "/v1/uploads/inline",
        json!({"filename": format!("{CANARY}.png"), "content_base64": "iVBORw0KGgo="}),
    ));
    push(call("GET", "/v1/uploads", json!({})));
    // brief de uma Run
    push(call(
        "POST",
        &format!("/v1/projects/{pid}/runs"),
        json!({"brief_text": format!("brief {CANARY}"), "start": false}),
    ));
    // idempotência e id de requisição
    push(send(
        &s,
        "POST",
        "/v1/projects",
        Some(&s.admin),
        &[
            ("Content-Type", "application/json"),
            ("Idempotency-Key", CANARY),
            ("X-Request-Id", CANARY),
        ],
        json!({"name": "idem"}).to_string().as_bytes(),
    ));
    // caminho, query, parâmetros inventados, erros
    push(send(
        &s,
        "GET",
        &format!("/v1/projects/{CANARY}"),
        Some(&s.admin),
        &[],
        b"",
    ));
    push(send(
        &s,
        "GET",
        &format!("/v1/projects/{CANARY}/summary"),
        Some(&s.admin),
        &[],
        b"",
    ));
    push(send(
        &s,
        "GET",
        &format!("/v1/projects?after={CANARY}&{CANARY}=1"),
        Some(&s.admin),
        &[],
        b"",
    ));
    push(send(
        &s,
        "GET",
        &format!("/v1/{CANARY}"),
        Some(&s.admin),
        &[],
        b"",
    ));
    push(send(
        &s,
        "DELETE",
        &format!("/v1/uploads/{CANARY}"),
        Some(&s.admin),
        &[],
        b"",
    ));
    push(send(
        &s,
        "DELETE",
        &format!("/v1/tokens/{CANARY}"),
        Some(&s.admin),
        &[],
        b"",
    ));
    push(call(
        "POST",
        "/v1/projects",
        json!({"name": "x", CANARY: CANARY}),
    ));
    push(call(
        "POST",
        "/v1/tokens",
        json!({"name": CANARY, "scopes": [CANARY]}),
    ));
    push(send(
        &s,
        "POST",
        "/v1/projects",
        Some(&s.admin),
        &json_h,
        format!("{{\"name\": \"{CANARY}\"").as_bytes(),
    ));
    push(send(&s, "GET", "/v1/server", Some(CANARY), &[], b""));
    push(send(
        &s,
        "GET",
        "/v1/health",
        None,
        &[("Origin", CANARY)],
        b"",
    ));
    push(send(
        &s,
        "POST",
        "/v1/webhooks",
        Some(&s.admin),
        &[("Content-Type", CANARY)],
        b"{}",
    ));
    push(Some(
        try_parse(
            &exchange(
                s.addr,
                format!("GET /v1/health HTTP/1.1\r\nHost: {CANARY}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
                std::time::Duration::from_secs(5),
            )
            .unwrap(),
        )
        .unwrap(),
    ));
    push(Some(
        try_parse(
            &exchange(
                s.addr,
                format!("{CANARY} /v1/health HTTP/1.1\r\n\r\n").as_bytes(),
                std::time::Duration::from_secs(5),
            )
            .unwrap(),
        )
        .unwrap(),
    ));
    assert!(seen.len() >= 25);
    assert_clean("responses", &all_text(&seen));

    // as superfícies de leitura que refletem estado/trilha
    let mut reads: Vec<Resp> = Vec::new();
    for p in [
        "/v1/audit?limit=500",
        "/v1/events?limit=500",
        "/v1/projects",
        "/v1/webhooks",
        "/v1/uploads",
        "/v1/tokens",
        "/v1/openapi.json",
        "/v1/server",
    ] {
        reads.push(send(&s, "GET", p, Some(&s.admin), &[], b"").unwrap());
    }
    reads.push(s.call("GET", &format!("/v1/projects/{pid}/history"), None));
    reads.push(s.call("GET", &format!("/v1/projects/{pid}/summary"), None));
    assert_clean("read surfaces", &all_text(&reads));
    // stream SSE (lê por ~1,5 s a partir do começo)
    {
        use std::io::{Read, Write};
        let mut sock = std::net::TcpStream::connect(s.addr).unwrap();
        sock.set_read_timeout(Some(std::time::Duration::from_millis(400)))
            .unwrap();
        write!(
            sock,
            "GET /v1/events/stream?after=0 HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\n\r\n",
            s.addr.port(),
            s.admin
        )
        .unwrap();
        let mut got = Vec::new();
        let t0 = std::time::Instant::now();
        let mut buf = [0u8; 4096];
        while t0.elapsed() < std::time::Duration::from_millis(1500) {
            if let Ok(n) = sock.read(&mut buf) {
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
            }
        }
        assert!(String::from_utf8_lossy(&got).starts_with("HTTP/1.1 200"));
        assert!(
            !contains(&got, CANARY.as_bytes()),
            "the canary leaked into the SSE stream"
        );
    }
    // a trilha de auditoria e o banco: estrutural (linhas) e bytes (arquivos, WAL inclusive)
    for row in s.core().db.audit_list(0, 5000).unwrap() {
        let t = serde_json::to_string(&row).unwrap();
        assert!(!t.contains(CANARY), "{t}");
    }
    assert!(
        files_containing(s.dir.path(), CANARY.as_bytes()).is_empty(),
        "{:?}",
        files_containing(s.dir.path(), CANARY.as_bytes())
    );
    assert!(
        names_containing(s.dir.path(), CANARY).is_empty(),
        "{:?}",
        names_containing(s.dir.path(), CANARY)
    );
    // server.json é a única descoberta e não carrega segredo nenhum
    let info = std::fs::read_to_string(s.dir.path().join("server.json")).unwrap();
    assert!(!info.contains(CANARY) && !info.contains(&s.admin));
    // as formas codificadas (base64/percent) também estão registradas
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(CANARY);
    let r = s.call("POST", "/v1/projects", Some(json!({"name": b64.clone()})));
    assert!(!whole(&r).contains(&b64), "base64 form leaked");
}

#[test]
fn a_server_minted_token_is_a_canary_too() {
    let s = start("canary-token", |c| c.rate_scale = 100.0);
    let t = s.token(
        "minted",
        &[
            "project:write",
            "project:read",
            "media:write",
            "webhook:manage",
        ],
    );
    let mut seen = vec![];
    let jh = [("Content-Type", "application/json")];
    // o próprio segredo como nome/descrição/arquivo/caminho: nada volta nem fica no disco
    seen.extend(send(
        &s,
        "POST",
        "/v1/projects",
        Some(&t),
        &jh,
        json!({"name": t}).to_string().as_bytes(),
    ));
    seen.extend(send(
        &s,
        "POST",
        "/v1/webhooks",
        Some(&t),
        &jh,
        json!({"url": "https://example.com/h", "events": ["*"], "description": t})
            .to_string()
            .as_bytes(),
    ));
    seen.extend(upload(&s, &t, &pct(&format!("{t}.png")), &tiny_png(), &[]));
    seen.extend(send(
        &s,
        "GET",
        &format!("/v1/projects/{t}"),
        Some(&t),
        &[],
        b"",
    ));
    seen.extend(send(
        &s,
        "GET",
        "/v1/projects",
        Some(&t),
        &[("X-Request-Id", &t)],
        b"",
    ));
    seen.extend(send(
        &s,
        "POST",
        "/v1/projects",
        Some(&t),
        &[
            ("Content-Type", "application/json"),
            ("Idempotency-Key", &t),
        ],
        b"{\"name\":\"k\"}",
    ));
    for r in &seen {
        assert!(
            !whole(r).contains(&t),
            "the token leaked into a response:\n{}",
            whole(r)
        );
    }
    let list = s.call("GET", "/v1/audit?limit=500", None);
    assert!(!whole(&list).contains(&t));
    assert!(files_containing(s.dir.path(), t.as_bytes()).is_empty());
    assert!(names_containing(s.dir.path(), &t).is_empty());
}

#[test]
fn a_panic_message_carrying_the_canary_is_written_redacted() {
    use std::sync::{Arc, Mutex};
    static HOOK: Mutex<()> = Mutex::new(());
    let _g = HOOK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    capia_secrets::register_global(CANARY);
    let out = Arc::new(Mutex::new(String::new()));
    let o = out.clone();
    let prev = std::panic::take_hook();
    capia_secrets::install_panic_hook_with(Box::new(move |m| o.lock().unwrap().push_str(m)));
    let r = std::panic::catch_unwind(|| panic!("handler failed while holding {CANARY}"));
    std::panic::set_hook(prev);
    assert!(r.is_err());
    let text = out.lock().unwrap().clone();
    assert!(!text.is_empty() && !text.contains(CANARY), "{text}");
}

#[test]
fn a_real_server_process_prints_the_bootstrap_token_once_and_never_leaks_it_elsewhere() {
    let dir = TempDir::new("canary-child");
    let mut srv = spawn_server(dir.path(), &["--bootstrap"]);
    srv.wait_ready();
    let tok = srv
        .bootstrap
        .clone()
        .expect("--bootstrap must print the token");
    assert!(tok.starts_with("capia_") && tok.len() == 70);
    // empurra o token (agora conhecido do processo filho) por todos os caminhos de eco
    let mut seen: Vec<Resp> = Vec::new();
    let jh = [("Content-Type", "application/json")];
    seen.extend(srv.json_call(
        "POST",
        "/v1/projects",
        &tok,
        &json!({"name": format!("n {tok}")}),
    ));
    seen.extend(srv.json_call("POST", "/v1/webhooks", &tok, &json!({"url": format!("https://example.com/h?k={tok}"), "events": ["*"], "description": tok})));
    seen.extend(srv.call("GET", &format!("/v1/projects/{tok}"), Some(&tok), &[], b""));
    seen.extend(srv.call(
        "GET",
        "/v1/server",
        Some(&tok),
        &[("X-Request-Id", &tok)],
        b"",
    ));
    seen.extend(srv.call(
        "POST",
        "/v1/projects",
        Some(&tok),
        &[
            ("Content-Type", "application/json"),
            ("Idempotency-Key", &tok),
        ],
        b"{\"name\":\"i\"}",
    ));
    seen.extend(srv.call(
        "POST",
        "/v1/uploads",
        Some(&tok),
        &[("X-Capia-Filename", &pct(&format!("{tok}.png")))],
        &tiny_png(),
    ));
    seen.extend(srv.call(
        "POST",
        "/v1/projects",
        Some(&tok),
        &jh,
        format!("{{bad {tok}").as_bytes(),
    ));
    seen.extend(srv.call("GET", "/v1/events?limit=500", Some(&tok), &[], b""));
    seen.extend(srv.call("GET", "/v1/audit?limit=500", Some(&tok), &[], b""));
    for r in &seen {
        assert!(
            !whole(r).contains(&tok),
            "token leaked in a response:\n{}",
            whole(r)
        );
    }
    srv.stop();
    // stdout: exatamente UMA linha com o token — a do bootstrap — e nada mais
    let out = srv.out();
    let with: Vec<&str> = out.lines().filter(|l| l.contains(&tok)).collect();
    assert_eq!(with.len(), 1, "stdout: {out}");
    assert_eq!(with[0], format!("CAPIA_BOOTSTRAP_TOKEN={tok}"));
    // stderr: nenhuma ocorrência
    let err = srv.err();
    assert!(!err.contains(&tok), "stderr leaked the token:\n{err}");
    assert!(!err.contains("panicked"), "the server panicked:\n{err}");
    // disco: só o hash
    let hits = files_containing(dir.path(), tok.as_bytes());
    assert!(hits.is_empty(), "{hits:?}");
    assert!(names_containing(dir.path(), &tok).is_empty());
}

#[test]
fn error_bodies_redact_secrets_even_when_the_message_carries_one() {
    use capia_server::error::ApiErr;
    capia_secrets::register_global(CANARY);
    // defesa em profundidade: mesmo que uma mensagem/detalhe carregue o segredo (um bug futuro de
    // handler), o corpo público da API nunca o mostra
    let e = ApiErr::bad_request(format!("failed for {CANARY}")).with_details(json!({
        "echo": CANARY, "nested": {"list": [CANARY, "ok"]}
    }));
    let body = e.body("req_x").to_string();
    assert!(!body.contains(CANARY), "{body}");
    assert!(body.contains("[REDACTED]"));
    assert!(
        body.contains("\"ok\""),
        "unrelated details must survive: {body}"
    );
}
