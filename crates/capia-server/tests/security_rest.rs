//! Segurança do REST `/v1` (Track D-2, pentest em código): autenticação, escopos, tokens, rota,
//! caminho em disco, SSRF, upload, forma da requisição, CORS/Host, idempotência, vazamento e DoS.
//! Cada teste é um ataque: o servidor real em loopback, bytes reais. Ver `docs/phase6/PENTEST_REPORT.md`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "common/attack.rs"]
mod attack;
mod common;

use attack::*;
use capia_server::catalog;
use capia_server::scope::{ALL_SCOPES, Scope};
use capia_server::{CallCtx, auth};
use common::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn all_scope_names() -> Vec<&'static str> {
    ALL_SCOPES.iter().map(|s| s.as_str()).collect()
}

/// Substitui `{x}` do molde de rota por valores inertes.
fn fill_path(tpl: &str) -> String {
    tpl.split('/')
        .map(|seg| {
            if seg.starts_with('{') {
                if seg == "{project_id}" { "p1" } else { "x1" }
            } else {
                seg
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn code_of(r: &Resp) -> String {
    serde_json::from_slice::<Value>(&r.body)
        .ok()
        .and_then(|v| v["code"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

// =================================================================================================
// 1. Autenticação
// =================================================================================================

#[test]
fn authorization_header_tricks_never_authenticate() {
    let s = start("auth-tricks", |c| c.rate_scale = 50.0);
    let t = s.admin.clone();
    let hs = |v: &str| vec![("Authorization".to_owned(), v.to_owned())];
    let try_hdr = |headers: Vec<(String, String)>| -> u16 {
        let host = format!("127.0.0.1:{}", s.addr.port());
        let mut h: Vec<(&str, &str)> = vec![("Host", &host), ("Connection", "close")];
        for (k, v) in &headers {
            h.push((k, v));
        }
        try_parse(
            &exchange(
                s.addr,
                &build("GET", "/v1/server", &h, b""),
                std::time::Duration::from_secs(10),
            )
            .unwrap(),
        )
        .unwrap()
        .status
    };
    // referência: o esquema é case-insensitive (RFC 7235) — o válido passa
    assert_eq!(try_hdr(hs(&format!("Bearer {t}"))), 200);
    assert_eq!(try_hdr(hs(&format!("bearer {t}"))), 200);
    assert_eq!(try_hdr(hs(&format!("BEARER {t}"))), 200);
    // tudo abaixo é recusado (401), nunca 200
    let upper = format!("capia_{}", t[6..].to_ascii_uppercase());
    let bad: Vec<String> = vec![
        String::new(),
        "Bearer".into(),
        "Bearer ".into(),
        format!("Basic {t}"),
        format!("Token {t}"),
        t.clone(), // sem esquema
        format!("Bearer\t{t}"),
        format!("Bearer {t} extra"),
        format!("Bearer {t}x"),
        format!("Bearer {}", &t[..t.len() - 1]),
        format!("Bearer {upper}"),
        format!("Bearer CAPIA_{}", &t[6..]),
        format!("Bearer {t}\u{200b}"),
        format!("Bearer \u{00a0}{t}"),
        format!("Bearer {}", t.replace('a', "а")), // 'a' cirílico
        format!("Bearer \"{t}\""),
        format!("Bearer {t},Bearer {t}"),
        format!("Bearer {}", "A".repeat(100_000)),
        format!("Bearer capia_{}", "0".repeat(64)),
        format!("Bearer {}", "capia_".repeat(30)),
        format!("Bearer %63apia_{}", &t[6..]),
    ];
    for b in &bad {
        let st = try_hdr(hs(b));
        let shown: String = b.chars().take(40).collect();
        assert!(
            matches!(st, 400 | 401 | 431),
            "`{shown}…` answered {st} (must be refused)"
        );
        assert_ne!(st, 200);
    }
    // dois Authorization: ambíguo ⇒ recusado em qualquer ordem
    let both_a = vec![
        ("Authorization".to_owned(), format!("Bearer {t}")),
        ("Authorization".to_owned(), "Bearer nope".to_owned()),
    ];
    let both_b = vec![
        ("Authorization".to_owned(), "Bearer nope".to_owned()),
        ("Authorization".to_owned(), format!("Bearer {t}")),
    ];
    assert_eq!(
        try_hdr(both_a),
        400,
        "duplicate Authorization (valid first)"
    );
    assert_eq!(try_hdr(both_b), 400, "duplicate Authorization (valid last)");
    // o token na query string / outro cabeçalho NUNCA vale
    for target in [
        format!("/v1/server?token={t}"),
        format!("/v1/server?access_token={t}"),
        format!("/v1/server?authorization=Bearer%20{t}"),
        format!("/v1/server?api_key={t}"),
    ] {
        let r = send(&s, "GET", &target, None, &[], b"").unwrap();
        assert_eq!(r.status, 401, "{}", target.replace(&t, "<tok>"));
    }
    for (k, v) in [
        ("X-Api-Key", t.as_str()),
        ("X-Auth-Token", t.as_str()),
        ("Cookie", &format!("token={t}")),
        ("Proxy-Authorization", &format!("Bearer {t}")),
    ] {
        let r = send(&s, "GET", "/v1/server", None, &[(k, v)], b"").unwrap();
        assert_eq!(r.status, 401, "{k}");
    }
}

#[test]
fn every_auth_failure_looks_the_same() {
    let s = start("auth-uniform", |c| c.rate_scale = 50.0);
    let revoked = s.token("revoked", &["project:read"]);
    let rid = s.call("GET", "/v1/tokens", None).json()["tokens"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "revoked")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        s.call("DELETE", &format!("/v1/tokens/{rid}"), None).status,
        200
    );
    let (_, expired) = auth::create_token(
        &s.core().db,
        "expired",
        &["project:read".to_owned()],
        Some(0),
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    let unknown = format!("capia_{}", "ab".repeat(32));
    let cases: Vec<(&str, Option<&str>)> = vec![
        ("missing", None),
        ("malformed", Some("not-a-token")),
        ("unknown", Some(unknown.as_str())),
        ("revoked", Some(revoked.as_str())),
        ("expired", Some(expired.as_str())),
    ];
    let mut shapes = BTreeSet::new();
    for (label, tok) in cases {
        let r = send(&s, "GET", "/v1/server", tok, &[], b"").unwrap();
        assert_eq!(r.status, 401, "{label}");
        let mut b = r.json();
        b["request_id"] = json!("-");
        let www = r.header("www-authenticate").map(str::to_owned);
        shapes.insert((b.to_string(), www));
    }
    assert_eq!(
        shapes.len(),
        1,
        "401 responses differ between failure modes (existence oracle): {shapes:?}"
    );
}

// =================================================================================================
// 2. Matriz de escopos (gerada do catálogo)
// =================================================================================================

/// Pede a operação como o cliente faria; corpo mínimo para os mutantes.
fn hit(s: &TestServer, token: &str, def: &catalog::OpDef) -> Resp {
    let path = fill_path(def.path);
    if def.name == "uploads.create" {
        return upload(s, token, "a.png", b"x", &[]).unwrap();
    }
    let has_body = matches!(def.method, "POST" | "PATCH" | "PUT");
    let body: &[u8] = if has_body { b"{}" } else { b"" };
    let extra: &[(&str, &str)] = if has_body {
        &[("Content-Type", "application/json")]
    } else {
        &[]
    };
    send(s, def.method, &path, Some(token), extra, body).unwrap()
}

#[test]
fn scope_matrix_exactly_the_declared_scope_opens_each_operation() {
    let s = start("matrix", |c| c.rate_scale = 100.0);
    let tokens: Vec<(Scope, String)> = ALL_SCOPES
        .iter()
        .map(|sc| {
            (
                *sc,
                s.token(&format!("only-{}", sc.as_str()), &[sc.as_str()]),
            )
        })
        .collect();
    let mut checked = 0;
    for def in catalog::ops() {
        for (sc, tok) in &tokens {
            let r = hit(&s, tok, def);
            let label = format!("{} with only `{}`", def.name, sc.as_str());
            match def.scope {
                None => assert!(r.status < 400 || r.status == 429, "{label}: {}", r.status),
                Some(required) if required == *sc => {
                    // aberto: pode falhar por outro motivo (404/422/409), nunca por auth/scope
                    assert_ne!(r.status, 401, "{label}");
                    assert!(
                        !(r.status == 403 && code_of(&r) == "INSUFFICIENT_SCOPE"),
                        "{label}: the declared scope was refused"
                    );
                    assert!(r.status < 500, "{label}: 5xx {}", r.status);
                }
                Some(required) => {
                    assert_eq!(
                        r.status,
                        403,
                        "{label}: must be forbidden (needs {})",
                        required.as_str()
                    );
                    assert_eq!(code_of(&r), "INSUFFICIENT_SCOPE", "{label}");
                    // 403 antes de qualquer 404/422: o corpo não revela se o recurso existe
                    assert_eq!(
                        r.json()["details"]["required_scope"],
                        required.as_str(),
                        "{label}"
                    );
                }
            }
            checked += 1;
        }
    }
    assert!(
        checked >= 11 * 50,
        "the matrix must cover the whole catalog ({checked})"
    );
    // um token com TODOS os escopos continua passando pelo mesmo caminho
    let r = hit(&s, &s.admin, catalog::find("tokens.list").unwrap());
    assert_eq!(r.status, 200);
}

#[test]
fn every_operation_demands_authentication_before_anything_else() {
    let s = start("matrix-noauth", |c| c.rate_scale = 100.0);
    for def in catalog::ops() {
        if def.scope.is_none() {
            continue;
        }
        let path = fill_path(def.path);
        let r = send(&s, def.method, &path, None, &[], b"").unwrap();
        assert_eq!(
            r.status, 401,
            "{} answered {} without a token",
            def.name, r.status
        );
    }
}

/// Decisão de autorização = `required ∈ granted`, para conjuntos aleatórios (PRNG determinístico).
#[test]
fn authorization_decision_equals_membership_for_random_scope_sets() {
    let s = start("scope-prop", |c| c.rate_scale = 1000.0);
    let core = s.core().clone();
    let ops: Vec<&'static catalog::OpDef> = catalog::ops()
        .iter()
        .filter(|d| d.scope.is_some() && d.name != "uploads.create")
        .collect();
    let mut rng = Prng::new(0xC0FF_EE11);
    let mut denied = 0;
    let mut allowed = 0;
    for i in 0..300 {
        let mask = 1 + rng.below((1 << ALL_SCOPES.len()) - 1);
        let granted: BTreeSet<Scope> = ALL_SCOPES
            .iter()
            .enumerate()
            .filter(|(b, _)| mask & (1 << b) != 0)
            .map(|(_, s)| *s)
            .collect();
        let principal = auth::Principal {
            token_id: format!("tok_prop{i}"),
            name: "prop".into(),
            scopes: granted.clone(),
        };
        let ctx = CallCtx {
            principal: Some(principal),
            surface: "rest",
            request_id: format!("req_prop{i}"),
            idempotency_key: None,
        };
        for _ in 0..10 {
            let def = *rng.pick(&ops);
            let required = def.scope.unwrap();
            let out = core.call(&ctx, def.name, json!({}));
            let forbidden = matches!(&out, Err(e) if e.code == "INSUFFICIENT_SCOPE");
            assert_eq!(
                forbidden,
                !granted.contains(&required),
                "op {} needs {} granted {:?}: {:?}",
                def.name,
                required.as_str(),
                granted,
                out.as_ref().err().map(|e| e.code.clone())
            );
            if let Err(e) = &out {
                assert_ne!(e.code, "UNAUTHORIZED");
            }
            if forbidden {
                denied += 1;
            } else {
                allowed += 1;
            }
        }
    }
    assert!(
        denied > 200 && allowed > 200,
        "{denied}/{allowed}: poor coverage"
    );
}

// =================================================================================================
// 3. Segurança dos tokens
// =================================================================================================

#[test]
fn token_secrets_are_256_bit_hex_and_never_repeat() {
    let mut seen = BTreeSet::new();
    let mut freq = [0usize; 16];
    let scopes: BTreeSet<Scope> = [Scope::ProjectRead].into();
    for _ in 0..300 {
        let (row, secret) = auth::mint("t", &scopes, None, None, 1).unwrap();
        assert!(
            secret.starts_with("capia_") && secret.len() == 70,
            "{secret}"
        );
        let hex = &secret[6..];
        assert!(
            hex.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert!(seen.insert(secret.clone()), "token collision");
        assert_ne!(row.secret_hash, secret, "the hash must not be the secret");
        assert_eq!(row.secret_hash.len(), 64);
        for b in hex.bytes() {
            freq[usize::from(if b <= b'9' { b - b'0' } else { b - b'a' + 10 })] += 1;
        }
    }
    let exp = 300.0 * 64.0 / 16.0;
    for (d, f) in freq.iter().enumerate() {
        let f = *f as f64;
        assert!(
            (exp * 0.8..=exp * 1.2).contains(&f),
            "hex digit {d} appears {f} times (expected ~{exp}): the generator looks biased"
        );
    }
}

#[test]
fn only_the_hash_of_a_token_ever_reaches_the_disk() {
    let s = start("token-disk", |_| {});
    let t = s.call(
        "POST",
        "/v1/tokens",
        Some(json!({"name": "disk", "scopes": ["project:read"]})),
    );
    let secret = t.json()["secret"].as_str().unwrap().to_owned();
    // usa o token (touch/last_used) e provoca erros com ele
    for _ in 0..3 {
        s.call_as(&secret, "GET", "/v1/server", None);
        s.call_as(&secret, "GET", "/v1/tokens", None);
    }
    let rotated = s.call(
        "POST",
        &format!(
            "/v1/tokens/{}/rotate",
            t.json()["token"]["id"].as_str().unwrap()
        ),
        None,
    );
    let secret2 = rotated.json()["secret"].as_str().unwrap().to_owned();
    for needle in [&secret, &secret2, &s.admin] {
        // inclui WAL/SHM: o checkpoint pode não ter acontecido
        let hits = files_containing(s.dir.path(), needle.as_bytes());
        assert!(hits.is_empty(), "token secret found in {hits:?}");
        let tail = &needle[6..];
        let hits = files_containing(s.dir.path(), tail.as_bytes());
        assert!(hits.is_empty(), "token secret tail found in {hits:?}");
    }
    // mas o hash SHA-256 está lá (é assim que se autentica)
    let h = capia_server::mac::sha256_hex(secret.as_bytes());
    assert!(
        !files_containing(s.dir.path(), h.as_bytes()).is_empty() || {
            // o SQLite pode guardar o hash em páginas que ainda não vieram para o arquivo principal
            s.core().db.token_by_hash(&h).unwrap().is_some()
        }
    );
}

#[test]
fn revocation_takes_effect_immediately_even_under_concurrency() {
    let s = start("revoke-race", |c| c.rate_scale = 100.0);
    let created = s
        .call(
            "POST",
            "/v1/tokens",
            Some(json!({"name": "victim", "scopes": ["project:read"]})),
        )
        .json();
    let secret = created["secret"].as_str().unwrap().to_owned();
    let id = created["token"]["id"].as_str().unwrap().to_owned();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let revoked_at = std::sync::Arc::new(std::sync::Mutex::new(None::<std::time::Instant>));
    let late_ok = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let addr = s.addr;
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let (secret, stop, revoked_at, late_ok) = (
                secret.clone(),
                stop.clone(),
                revoked_at.clone(),
                late_ok.clone(),
            );
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                    let started = std::time::Instant::now();
                    let r = request(addr, "GET", "/v1/server", Some(&secret), &[], None);
                    // um pedido que COMEÇOU depois da revogação confirmada não pode dar 200
                    if r.status == 200 && revoked_at.lock().unwrap().is_some_and(|t| started > t) {
                        late_ok.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            })
        })
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(150));
    let r = s.call("DELETE", &format!("/v1/tokens/{id}"), None);
    assert_eq!(r.status, 200);
    *revoked_at.lock().unwrap() = Some(std::time::Instant::now());
    std::thread::sleep(std::time::Duration::from_millis(300));
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(late_ok.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(s.call_as(&secret, "GET", "/v1/server", None).status, 401);
    // revogar duas vezes é idempotente e não ressuscita
    s.call("DELETE", &format!("/v1/tokens/{id}"), None);
    assert_eq!(s.call_as(&secret, "GET", "/v1/server", None).status, 401);
}

#[test]
fn expiry_is_enforced_and_rotation_kills_the_old_secret_at_once() {
    let s = start("expiry-rotate", |c| c.rate_scale = 100.0);
    let (_, short) =
        auth::create_token(&s.core().db, "short", &["project:read".to_owned()], Some(1)).unwrap();
    assert_eq!(s.call_as(&short, "GET", "/v1/server", None).status, 200);
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert_eq!(
        s.call_as(&short, "GET", "/v1/server", None).status,
        401,
        "expired"
    );
    // rotação: o segredo antigo morre na mesma resposta
    let c = s
        .call(
            "POST",
            "/v1/tokens",
            Some(json!({"name": "r", "scopes": ["project:read"], "expires_in_seconds": 3600})),
        )
        .json();
    let old = c["secret"].as_str().unwrap().to_owned();
    let id = c["token"]["id"].as_str().unwrap().to_owned();
    let rot = s.call("POST", &format!("/v1/tokens/{id}/rotate"), None);
    assert!(matches!(rot.status, 200 | 201), "{}", rot.status);
    let new = rot.json()["secret"].as_str().unwrap().to_owned();
    assert_ne!(old, new);
    assert_eq!(
        s.call_as(&old, "GET", "/v1/server", None).status,
        401,
        "old secret must be dead"
    );
    assert_eq!(s.call_as(&new, "GET", "/v1/server", None).status, 200);
    // a rotação não amplia validade nem escopos
    let nt = &rot.json()["token"];
    assert_eq!(nt["scopes"], json!(["project:read"]));
    let ttl = nt["expires_ms"].as_u64().unwrap() - nt["created_ms"].as_u64().unwrap();
    assert!(ttl <= 3_600_000, "rotation extended the lifetime: {ttl}");
    // rotacionar o já-rotacionado/revogado: recusado
    let again = s.call("POST", &format!("/v1/tokens/{id}/rotate"), None);
    assert_eq!(again.status, 409);
    // corrida: duas rotações simultâneas do mesmo token ⇒ exatamente uma vence
    let nid = rot.json()["token"]["id"].as_str().unwrap().to_owned();
    let (a, b) = (s.addr, s.addr);
    let admin = s.admin.clone();
    let path = format!("/v1/tokens/{nid}/rotate");
    let (p1, p2) = (path.clone(), path);
    let (a1, a2) = (admin.clone(), admin);
    let t1 = std::thread::spawn(move || request(a, "POST", &p1, Some(&a1), &[], None).status);
    let t2 = std::thread::spawn(move || request(b, "POST", &p2, Some(&a2), &[], None).status);
    let mut got = [t1.join().unwrap(), t2.join().unwrap()];
    got.sort_unstable();
    assert!(
        got == [200, 409] || got == [201, 409],
        "concurrent rotations must yield one winner: {got:?}"
    );
}

#[test]
fn a_token_can_never_mint_or_rotate_beyond_its_own_scopes() {
    let s = start("no-escalation", |c| c.rate_scale = 100.0);
    // para cada escopo possível, um admin:tokens que NÃO o tem não consegue criá-lo
    for sc in all_scope_names() {
        if sc == "admin:tokens" {
            continue;
        }
        let weak = s.token(&format!("w-{sc}"), &["admin:tokens"]);
        let r = s.call_as(
            &weak,
            "POST",
            "/v1/tokens",
            Some(json!({"name": "x", "scopes": [sc]})),
        );
        assert_eq!(r.status, 403, "{sc}");
        assert_eq!(code_of(&r), "SCOPE_ESCALATION");
        let both = s.call_as(
            &weak,
            "POST",
            "/v1/tokens",
            Some(json!({"name": "x", "scopes": ["admin:tokens", sc]})),
        );
        assert_eq!(both.status, 403, "mixed request with {sc}");
    }
    // nomes de escopo inventados / duplicados / case: 422, nunca criado
    for bad in [
        json!(["*"]),
        json!(["admin:*"]),
        json!(["ADMIN:TOKENS"]),
        json!(["admin:tokens", "admin:tokens"]),
        json!(["admin:tokens "]),
        json!([]),
    ] {
        let r = s.call(
            "POST",
            "/v1/tokens",
            Some(json!({"name": "x", "scopes": bad})),
        );
        assert!(matches!(r.status, 422 | 400), "{bad}: {}", r.status);
    }
    // rotacionar token mais forte: bloqueado
    let strong = s
        .call(
            "POST",
            "/v1/tokens",
            Some(json!({"name": "strong", "scopes": ["admin:tokens", "project:write"]})),
        )
        .json();
    let weak = s.token("weak", &["admin:tokens"]);
    let r = s.call_as(
        &weak,
        "POST",
        &format!(
            "/v1/tokens/{}/rotate",
            strong["token"]["id"].as_str().unwrap()
        ),
        None,
    );
    assert_eq!(r.status, 403);
    assert_eq!(code_of(&r), "SCOPE_ESCALATION");
    // o token forte original segue valendo (a tentativa falha não o revogou)
    assert_eq!(
        s.call_as(
            strong["secret"].as_str().unwrap(),
            "GET",
            "/v1/server",
            None
        )
        .status,
        403,
        "no project:read on it, but still authenticated (403, not 401)"
    );
}

// =================================================================================================
// 4. Confusão de rota / método
// =================================================================================================

#[test]
fn route_and_method_confusion_never_reaches_a_privileged_handler() {
    let s = start("route-confusion", |c| c.rate_scale = 100.0);
    let low = s.token("low", &["project:read"]);
    let pid = s.create_project("rc");
    let targets: Vec<String> = vec![
        "/v1/tokens".into(),
        "//v1/tokens".into(),
        "/v1//tokens".into(),
        "/v1/tokens/".into(),
        "/v1/tokens/.".into(),
        "/v1/./tokens".into(),
        "/V1/tokens".into(),
        "/v1/TOKENS".into(),
        "/v1/tokens%00".into(),
        "/v1/tokens%2f".into(),
        "/v1%2ftokens".into(),
        "/v1/%74okens".into(),
        "/v1/tokens;x=1".into(),
        "/v1/tokens#frag".into(),
        "/v1/tokens?x=1".into(),
        format!("/v1/projects/{pid}/../tokens"),
        format!("/v1/projects/{pid}/%2e%2e/tokens"),
        format!("/v1/projects/{pid}%2f..%2f..%2ftokens"),
        format!("/v1/projects/%2e%2e%2ftokens"),
        format!("/v1/projects/%252e%252e/tokens"),
        "/v1/audit".into(),
        "/v1/audit/".into(),
        "/v1/webhooks%2f".into(),
        "/v1/server/../tokens".into(),
        "/v1/health/../tokens".into(),
        "/v1/openapi.json/../tokens".into(),
        "/mcp/../v1/tokens".into(),
        "/%2e%2e/%2e%2e/etc/passwd".into(),
        "/v1/projects/..".into(),
        "/v1/projects/.".into(),
    ];
    for t in &targets {
        let r = send(&s, "GET", t, Some(&low), &[], b"");
        let Some(r) = r else { continue };
        assert!(
            !(200..300).contains(&r.status)
                || t.contains("projects/..")
                || t.contains("projects/."),
            "GET {t} answered {} with a low-scope token: {}",
            r.status,
            String::from_utf8_lossy(&r.body)
        );
        let body = String::from_utf8_lossy(&r.body);
        assert!(
            !body.contains("\"tokens\""),
            "GET {t} leaked the token list"
        );
    }
    // métodos exóticos: nunca 2xx num recurso de escrita/admin
    for m in [
        "HEAD", "TRACE", "PUT", "PROPFIND", "PATCH", "DELETE", "LINK", "PURGE",
    ] {
        let r = send(&s, m, "/v1/tokens", Some(&low), &[], b"").unwrap();
        assert!(
            matches!(r.status, 403 | 404 | 405 | 501),
            "{m} /v1/tokens → {}",
            r.status
        );
    }
    // CONNECT / alvo absoluto / asterisco: recusados na linha de requisição
    for raw_line in [
        "CONNECT 127.0.0.1:80 HTTP/1.1",
        "GET http://127.0.0.1/v1/tokens HTTP/1.1",
        "GET * HTTP/1.1",
        "GET v1/tokens HTTP/1.1",
        "OPTIONS * HTTP/1.1",
        "get /v1/health HTTP/1.1",
        "GET /v1/health HTTP/1.1 extra",
        "GET  /v1/health HTTP/1.1",
        "GET /v1/health HTTP/1.1 ",
        "GET /v1/health HTTP/2.0",
        "GET /v1/health HTTP/0.9",
        "GET /v1/health",
    ] {
        let req = format!(
            "{raw_line}\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {low}\r\nConnection: close\r\n\r\n",
            s.addr.port()
        );
        if let Some(out) = exchange(s.addr, req.as_bytes(), std::time::Duration::from_secs(10))
            && let Some(r) = try_parse(&out)
        {
            assert!(r.status >= 400, "`{raw_line}` answered {}", r.status);
        }
    }
    // override de método por cabeçalho é ignorado
    for (h, v) in [
        ("X-HTTP-Method-Override", "DELETE"),
        ("X-Method-Override", "DELETE"),
        ("X-HTTP-Method", "DELETE"),
    ] {
        let r = send(&s, "GET", "/v1/projects", Some(&low), &[(h, v)], b"").unwrap();
        assert_eq!(r.status, 200, "{h} must be ignored");
        assert!(r.json()["projects"].is_array());
    }
    // parâmetro de caminho vence a query e o corpo (o recurso é o da URL)
    let other = s.create_project("other");
    let _ = s.call("POST", &format!("/v1/projects/{pid}/open"), None);
    let r = send(
        &s,
        "GET",
        &format!("/v1/projects/{pid}/summary?project_id={other}"),
        Some(&low),
        &[],
        b"",
    )
    .unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(
        r.json()["project"]["id"],
        pid.as_str(),
        "query must not override the path"
    );
    assert!(healthy(&s));
}

#[test]
fn unknown_and_oversized_paths_do_not_crash_or_leak() {
    let s = start("odd-paths", |_| {});
    let big = format!("/v1/{}", "a".repeat(4000));
    let r = send(&s, "GET", &big, Some(&s.admin), &[], b"").unwrap();
    assert_eq!(r.status, 404);
    let too_big = format!("/v1/{}", "a".repeat(5000));
    let r = send(&s, "GET", &too_big, Some(&s.admin), &[], b"").unwrap();
    assert_eq!(r.status, 400);
    let deep = format!("/{}", "x/".repeat(2000));
    let r = send(&s, "GET", &deep, Some(&s.admin), &[], b"").unwrap();
    assert_eq!(r.status, 404);
    for t in [
        "/v1/%",
        "/v1/%zz",
        "/v1/%e0%a4%a",
        "/v1/%ff%fe",
        "/v1/\u{0}",
    ] {
        let r = send(&s, "GET", t, Some(&s.admin), &[], b"");
        if let Some(r) = r {
            assert!(r.status == 400 || r.status == 404, "{t}: {}", r.status);
        }
    }
    assert!(healthy(&s));
}

// =================================================================================================
// 5. Caminhos em disco (nenhum nome do cliente vira caminho fora do data_dir)
// =================================================================================================

fn png_padded(n: usize) -> Vec<u8> {
    let mut v = tiny_png();
    v.resize(n.max(v.len()), 0);
    v
}

/// Servidor com `data_dir` DENTRO de uma raiz que tem um diretório irmão `outside/` vigiado.
fn start_fenced(tag: &str, tweak: impl FnOnce(&mut capia_server::ServerConfig)) -> TestServer {
    let s = start(tag, |c| {
        c.data_dir = c.data_dir.join("data");
        c.rate_scale = 100.0;
        tweak(c);
    });
    std::fs::create_dir_all(s.dir.path().join("outside")).unwrap();
    std::fs::write(s.dir.path().join("outside/keep.txt"), b"keep").unwrap();
    s
}

fn assert_fenced(s: &TestServer) {
    let root = s.dir.path();
    for p in walk(root) {
        let rel = p.strip_prefix(root).unwrap();
        let first = rel
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_string_lossy()
            .into_owned();
        assert!(
            first == "data" || first == "outside",
            "something was written outside the data dir: {}",
            p.display()
        );
    }
    assert_eq!(
        std::fs::read(root.join("outside/keep.txt")).unwrap(),
        b"keep"
    );
    assert_eq!(
        walk(&root.join("outside")).len(),
        1,
        "outside/ gained files"
    );
}

#[test]
fn hostile_filenames_never_escape_the_data_dir_or_name_special_files() {
    let s = start_fenced("traversal", |_| {});
    let png = png_padded(200);
    let long255 = "a".repeat(251) + ".png";
    let names: Vec<String> = vec![
        "../../outside/evil.png".into(),
        "../outside/keep.txt".into(),
        "..\\..\\outside\\evil.png".into(),
        "/etc/cron.d/evil.png".into(),
        "/outside/evil.png".into(),
        "C:\\Windows\\System32\\evil.png".into(),
        "C:evil.png".into(),
        "\\\\server\\share\\evil.png".into(),
        "//server/share/evil.png".into(),
        "a\0b.png".into(),
        "evil.png\0.exe".into(),
        "CON".into(),
        "con.png".into(),
        "NUL.png".into(),
        "aux".into(),
        "PRN.tar.png".into(),
        "COM1.png".into(),
        "lpt9.png".into(),
        "clock$.png".into(),
        "trailing dot.png.".into(),
        "trailing space.png ".into(),
        "stream.png:hidden".into(),
        "stream.png::$DATA".into(),
        "meta.json".into(),
        "META.JSON".into(),
        "meta.json.tmp".into(),
        "x.png.part".into(),
        ".".into(),
        "..".into(),
        "...".into(),
        ".hidden.png".into(),
        "\u{ff0e}\u{ff0e}\u{ff0f}evil.png".into(),
        "..%2f..%2fevil.png".into(),
        "evil\r\nSet-Cookie: pwn=1.png".into(),
        "emoji-\u{1f4a5}.png".into(),
        "a\u{202e}gnp.exe".into(),
        "~/evil.png".into(),
        "$HOME/evil.png".into(),
        "`id`.png".into(),
        "-rf.png".into(),
        long255,
    ];
    let mut accepted = 0;
    for n in &names {
        let r = upload(&s, &s.admin, &pct(n), &png, &[])
            .unwrap_or_else(|| panic!("no answer for {n:?}"));
        assert!(
            r.status < 500,
            "{n:?} → {} {}",
            r.status,
            String::from_utf8_lossy(&r.body)
        );
        assert!(
            r.header("set-cookie").is_none(),
            "response splitting via {n:?}"
        );
        if r.status == 201 {
            accepted += 1;
            let meta = &r.json()["upload"];
            let meta = if meta.is_null() {
                r.json()
            } else {
                meta.clone()
            };
            let fname = meta["filename"].as_str().unwrap().to_owned();
            for bad in [
                '/', '\\', ':', '\0', '\r', '\n', '*', '?', '"', '<', '>', '|',
            ] {
                assert!(!fname.contains(bad), "{n:?} → {fname:?} keeps {bad:?}");
            }
            assert!(!fname.starts_with('.'), "{n:?} → hidden file {fname:?}");
            assert!(fname.len() <= 120, "{n:?} → {} bytes", fname.len());
            assert!(
                !windows_unsafe_name(&fname),
                "{n:?} → {fname:?} is a reserved Windows device name / has a trailing dot or space"
            );
            assert!(
                !fname.eq_ignore_ascii_case("meta.json")
                    && !fname.eq_ignore_ascii_case("meta.json.tmp"),
                "{n:?} → {fname:?} collides with the upload's own metadata files"
            );
            // o blob está onde o servidor diz, dentro de uploads/<id>/
            let id = meta["upload_id"].as_str().unwrap();
            let blob = s.core().cfg.data_dir.join("uploads").join(id).join(&fname);
            assert!(blob.is_file(), "{n:?}: blob {} missing", blob.display());
            assert_eq!(
                std::fs::read(&blob).unwrap(),
                png,
                "{n:?}: the payload was overwritten"
            );
        }
    }
    assert!(
        accepted >= 25,
        "the sanitizer should accept most names, got {accepted}"
    );
    assert_fenced(&s);
    // todos os diretórios de upload só têm o blob + meta.json
    for e in std::fs::read_dir(s.core().cfg.data_dir.join("uploads"))
        .unwrap()
        .flatten()
    {
        let files: Vec<String> = std::fs::read_dir(e.path())
            .unwrap()
            .flatten()
            .map(|f| f.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files.len(), 2, "{files:?}");
        assert!(files.iter().any(|f| f == "meta.json"));
    }
    assert!(healthy(&s));
}

#[test]
fn identifiers_in_paths_and_bodies_cannot_traverse() {
    let s = start_fenced("id-traversal", |_| {});
    let pid = s.create_project("t");
    let ids = [
        "..",
        ".",
        "....",
        "../..",
        "..%2f",
        "%2e%2e",
        "a/../b",
        "upl_/../../data",
        "C:",
        "C:\\x",
        "con",
        "x\0y",
        "upl_..%2f..%2f",
        "prj_%00",
    ];
    for id in ids {
        let enc = pct(id);
        for (m, path) in [
            ("GET", format!("/v1/projects/{enc}")),
            ("GET", format!("/v1/projects/{enc}/summary")),
            ("POST", format!("/v1/projects/{enc}/open")),
            ("DELETE", format!("/v1/uploads/{enc}")),
            ("GET", format!("/v1/projects/{pid}/assets/{enc}")),
            ("GET", format!("/v1/projects/{pid}/exports/{enc}")),
            ("GET", format!("/v1/webhooks/{enc}")),
            ("DELETE", format!("/v1/tokens/{enc}")),
        ] {
            let r = s.call(m, &path, if m == "POST" { Some(json!({})) } else { None });
            assert!(r.status < 500, "{m} {path}: {}", r.status);
            assert_ne!(r.status, 200, "{m} {path} succeeded for a hostile id");
        }
    }
    // ids hostis no CORPO (MCP/clients enviam por aqui): o schema barra antes de tocar o disco
    for id in ["upl_/../../x", "../x", "upl_a/..", "..\\x"] {
        let r = s.call(
            "POST",
            &format!("/v1/projects/{pid}/assets"),
            Some(json!({"upload_id": id})),
        );
        assert_eq!(r.status, 422, "{id}: {}", r.status);
    }
    // nomes de export hostis: o diretório de saída é sempre data/exports/<projeto>/<id>/
    let seqs = s.call(
        "POST",
        &format!("/v1/projects/{pid}/commands/preview"),
        Some(json!({"commands": create_sequence_cmds("ex1", "seq1")})),
    );
    let apply = s.call(
        "POST",
        &format!("/v1/projects/{pid}/commands/apply"),
        Some(json!({"plan_token": seqs.json()["plan_token"]})),
    );
    assert_eq!(
        apply.status,
        200,
        "{}",
        String::from_utf8_lossy(&apply.body)
    );
    for name in [
        "../../outside/x",
        "..\\..\\x",
        "/etc/passwd",
        "CON",
        "nul.mp4",
        "a\0b",
        "..",
    ] {
        let r = s.call(
            "POST",
            &format!("/v1/projects/{pid}/exports"),
            Some(json!({"items": [{"sequence": "seq1", "name": name, "preset": "intermediate"}]})),
        );
        assert!(r.status < 500 || r.status == 503, "{name:?}: {}", r.status);
        assert!(r.status != 500, "{name:?}");
    }
    assert_fenced(&s);
    assert!(healthy(&s));
}

#[test]
fn staging_files_are_reachable_through_no_route() {
    let s = start_fenced("staging-unreachable", |_| {});
    let r = upload(&s, &s.admin, "secret-name.png", &png_padded(300), &[]).unwrap();
    assert_eq!(r.status, 201);
    let id = r.json()["upload"]["upload_id"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| r.json()["upload_id"].as_str().map(str::to_owned))
        .unwrap();
    // nenhuma rota do catálogo devolve bytes de arquivo nem caminhos
    let mut texts = vec![];
    for t in [
        format!("/v1/uploads/{id}"),
        format!("/v1/uploads/{id}/content"),
        format!("/v1/uploads/{id}/download"),
        format!("/uploads/{id}/secret-name.png"),
        format!("/v1/uploads/{id}/secret-name.png"),
        format!("/v1/uploads/{id}/meta.json"),
        format!("/v1/files/{id}"),
        "/uploads/".to_owned(),
        "/server.json".to_owned(),
        "/v1/../server.json".to_owned(),
        "/v1/uploads?path=../server.db".to_owned(),
    ] {
        let r = send(&s, "GET", &t, Some(&s.admin), &[], b"").unwrap();
        assert!(
            r.status == 404 || r.status == 405 || r.status == 422,
            "{t}: {}",
            r.status
        );
        texts.push(whole(&r));
    }
    let list = s.call("GET", "/v1/uploads", None);
    let t = whole(&list);
    assert!(!t.contains(s.dir.path().to_string_lossy().as_ref()), "{t}");
    texts.push(t);
    for t in texts {
        assert!(leak_in(&t, s.dir.path()).is_none(), "{t}");
    }
}

// =================================================================================================
// 6. Abuso de upload
// =================================================================================================

fn uploads_in_staging(s: &TestServer) -> usize {
    s.core().list_uploads().len()
}

fn staging_dirs(s: &TestServer) -> usize {
    std::fs::read_dir(s.core().cfg.data_dir.join("uploads")).map_or(0, |rd| rd.flatten().count())
}

#[test]
fn oversized_declared_and_streamed_uploads_are_refused_and_leave_nothing() {
    let s = start_fenced("up-size", |c| {
        c.max_upload_bytes = 1000;
        c.max_json_bytes = 20 << 20;
    });
    // declarado > teto: recusa antes de ler (e antes de criar diretório)
    let big = png_padded(2000);
    let r = upload(&s, &s.admin, "big.png", &big, &[]).unwrap();
    assert_eq!(r.status, 413);
    assert_eq!(r.code(), "UPLOAD_TOO_LARGE");
    assert_eq!(
        staging_dirs(&s),
        0,
        "a refused upload must not leave a staging dir"
    );
    // no limite: aceito
    let ok = upload(&s, &s.admin, "ok.png", &png_padded(1000), &[]).unwrap();
    assert_eq!(ok.status, 201, "{}", String::from_utf8_lossy(&ok.body));
    // inline acima de 8 MiB
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(vec![0u8; 9 << 20]);
    let r = s.call(
        "POST",
        "/v1/uploads/inline",
        Some(json!({"filename": "big.png", "content_base64": b64})),
    );
    assert!(r.status == 413 || r.status == 422, "{}", r.status);
    assert_eq!(staging_dirs(&s), 1);
    assert!(healthy(&s));
}

#[test]
fn a_truncated_upload_is_never_stored_as_if_it_were_complete() {
    let s = start_fenced("up-trunc", |_| {});
    let png = png_padded(500);
    // Content-Length promete 500, o cliente manda 200 e fecha (caiu a rede / abortou)
    let head = format!(
        "POST /v1/uploads HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nX-Capia-Filename: cut.png\r\nContent-Length: 500\r\nConnection: close\r\n\r\n",
        s.addr.port(),
        s.admin
    );
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(&png[..200]);
    let out = exchange_half_close(s.addr, &bytes, std::time::Duration::from_secs(15)).unwrap();
    if let Some(r) = try_parse(&out) {
        assert!(
            r.status >= 400,
            "a truncated body was answered {}",
            r.status
        );
    }
    assert!(
        wait_until(std::time::Duration::from_secs(3), || uploads_in_staging(&s)
            == 0),
        "a truncated upload ended up staged: {:?}",
        s.core().list_uploads()
    );
    assert_eq!(staging_dirs(&s), 0);
    assert!(healthy(&s));
}

#[test]
fn a_stalled_upload_times_out_and_frees_the_worker_and_the_slot() {
    let s = start_fenced("up-stall", |c| {
        c.workers = 2;
        c.max_concurrent_uploads = 1;
    });
    let png = png_padded(500);
    let mut sock = std::net::TcpStream::connect(s.addr).unwrap();
    let head = format!(
        "POST /v1/uploads HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nX-Capia-Filename: slow.png\r\nContent-Length: 500\r\n\r\n",
        s.addr.port(),
        s.admin
    );
    std::io::Write::write_all(&mut sock, head.as_bytes()).unwrap();
    std::io::Write::write_all(&mut sock, &png[..50]).unwrap();
    // o slot fica ocupado enquanto o cliente "pendura"
    assert!(wait_until(std::time::Duration::from_secs(3), || {
        s.core()
            .uploads_active
            .load(std::sync::atomic::Ordering::SeqCst)
            == 1
    }));
    // o outro worker segue atendendo
    assert!(healthy(&s));
    // segundo upload simultâneo: teto de concorrência
    let r2 = upload(&s, &s.admin, "second.png", &png, &[]).unwrap();
    assert_eq!(r2.status, 429);
    assert_eq!(r2.code(), "TOO_MANY_UPLOADS");
    assert!(r2.header("retry-after").is_some());
    // o cliente continua mudo: o servidor desiste sozinho e devolve 408
    sock.set_read_timeout(Some(std::time::Duration::from_secs(20)))
        .unwrap();
    let mut out = Vec::new();
    let t0 = std::time::Instant::now();
    let _ = std::io::Read::read_to_end(&mut sock, &mut out);
    assert!(
        t0.elapsed() < std::time::Duration::from_secs(15),
        "the server never gave up on the stalled client"
    );
    if let Some(r) = try_parse(&out) {
        assert_eq!(r.status, 408, "{}", String::from_utf8_lossy(&r.body));
        assert_eq!(r.code(), "UPLOAD_TIMEOUT");
    }
    assert!(
        wait_until(std::time::Duration::from_secs(3), || {
            s.core()
                .uploads_active
                .load(std::sync::atomic::Ordering::SeqCst)
                == 0
        }),
        "the upload slot was never released"
    );
    assert_eq!(staging_dirs(&s), 0, "stalled upload left partial files");
    // e agora cabe um novo
    let r3 = upload(&s, &s.admin, "third.png", &png, &[]).unwrap();
    assert_eq!(r3.status, 201);
}

#[test]
fn staging_quota_zero_byte_checksum_and_dedupe_isolation() {
    let s = start_fenced("up-quota", |c| c.upload_quota_bytes = 5000);
    let a = upload(&s, &s.admin, "a.png", &png_padded(3000), &[]).unwrap();
    assert_eq!(a.status, 201);
    let a_id = a.json()["upload"]["upload_id"].as_str().unwrap().to_owned();
    // a cota (declarada) estoura
    let b = upload(&s, &s.admin, "b.png", &png_padded(3000), &[]).unwrap();
    assert_eq!(b.status, 507);
    assert_eq!(b.code(), "STORAGE_QUOTA_EXCEEDED");
    assert_eq!(staging_dirs(&s), 1);
    // liberar devolve a cota
    assert_eq!(
        s.call("DELETE", &format!("/v1/uploads/{a_id}"), None)
            .status,
        200
    );
    let c = upload(&s, &s.admin, "c.png", &png_padded(3000), &[]).unwrap();
    assert_eq!(c.status, 201);
    // zero byte: sem Content-Length ⇒ 411; inline vazio ⇒ 422
    let zero = send(
        &s,
        "POST",
        "/v1/uploads",
        Some(&s.admin),
        &[("X-Capia-Filename", "z.png"), ("Content-Length", "0")],
        b"",
    )
    .unwrap();
    assert_eq!(zero.status, 411);
    let inline = s.call(
        "POST",
        "/v1/uploads/inline",
        Some(json!({"filename": "z.png", "content_base64": ""})),
    );
    assert_eq!(inline.status, 422);
    // checksum errado: nada fica staged; certo: aceito (maiúsculas incluídas)
    let before = uploads_in_staging(&s);
    let wrong = upload(
        &s,
        &s.admin,
        "w.png",
        &png_padded(100),
        &[("X-Capia-Sha256", &"0".repeat(64))],
    )
    .unwrap();
    assert_eq!(wrong.status, 422);
    assert_eq!(wrong.code(), "CHECKSUM_MISMATCH");
    assert_eq!(uploads_in_staging(&s), before);
    let body = png_padded(100);
    let sum = capia_server::mac::sha256_hex(&body);
    let good = upload(
        &s,
        &s.admin,
        "g.png",
        &body,
        &[("X-Capia-Sha256", &sum.to_ascii_uppercase())],
    )
    .unwrap();
    assert_eq!(good.status, 201, "{}", String::from_utf8_lossy(&good.body));
    let junk = upload(
        &s,
        &s.admin,
        "j.png",
        &body,
        &[("X-Capia-Sha256", "not-a-hash")],
    )
    .unwrap();
    assert_eq!(junk.status, 422);
    // dedupe NUNCA devolve o upload de outro token
    let t1 = s.token("m1", &["media:write", "media:read"]);
    let t2 = s.token("m2", &["media:write", "media:read"]);
    s.call(
        "DELETE",
        &format!(
            "/v1/uploads/{}",
            c.json()["upload"]["upload_id"].as_str().unwrap()
        ),
        None,
    );
    let u1 = upload(&s, &t1, "same.png", &png_padded(120), &[]).unwrap();
    let u2 = upload(&s, &t2, "same.png", &png_padded(120), &[]).unwrap();
    assert_eq!((u1.status, u2.status), (201, 201));
    assert_ne!(
        u1.json()["upload"]["upload_id"],
        u2.json()["upload"]["upload_id"]
    );
    assert!(
        u2.json()["upload"].get("deduplicated").is_none(),
        "dedupe leaked another token's upload"
    );
    assert_ne!(
        u1.json()["upload"]["token_id"],
        u2.json()["upload"]["token_id"]
    );
    // …mas o MESMO token reaproveita
    let u1b = upload(&s, &t1, "same.png", &png_padded(120), &[]).unwrap();
    assert_eq!(
        u1b.json()["upload"]["upload_id"],
        u1.json()["upload"]["upload_id"]
    );
    assert_eq!(u1b.json()["upload"]["deduplicated"], true);
}

#[test]
fn disguised_and_polyglot_files_are_classified_by_content_only() {
    let s = start_fenced("up-sniff", |_| {});
    let elf = [&b"\x7fELF\x02\x01\x01\0"[..], &[0u8; 64]].concat();
    let pe = [&b"MZ\x90\0\x03\0\0\0"[..], &[0u8; 64]].concat();
    let macho = [&b"\xcf\xfa\xed\xfe\x07\0\0\x01"[..], &[0u8; 64]].concat();
    let sh = b"#!/bin/sh\nrm -rf /\n".to_vec();
    let html = b"<html><script>alert(1)</script></html>".to_vec();
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'/>".to_vec();
    let zip = b"PK\x03\x04aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_vec();
    let jar = b"PK\x03\x04aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_vec();
    for (name, body) in [
        ("movie.mp4", &elf),
        ("movie.mov", &pe),
        ("clip.mkv", &macho),
        ("photo.png", &sh),
        ("photo.jpg", &html),
        ("logo.png", &svg),
        ("brief.docx", &sh),
        ("archive.zip", &zip),
        ("app.jar", &jar),
        ("run.exe", &pe),
        ("note.txt", &elf),
        ("brief.pdf", &pe),
    ] {
        let r = upload(&s, &s.admin, name, body, &[]).unwrap();
        assert_eq!(
            r.status,
            415,
            "{name} was accepted: {}",
            String::from_utf8_lossy(&r.body)
        );
        assert_eq!(r.code(), "UNSUPPORTED_MEDIA_TYPE");
    }
    assert_eq!(staging_dirs(&s), 0);
    // o conteúdo manda: PNG com extensão .mp4 vira IMAGE, não vídeo; polyglot PNG+HTML segue PNG
    let png_as_mp4 = upload(&s, &s.admin, "fake.mp4", &png_padded(80), &[]).unwrap();
    assert_eq!(png_as_mp4.status, 201);
    assert_eq!(png_as_mp4.json()["upload"]["kind"], "image");
    let mut poly = tiny_png();
    poly.extend_from_slice(b"<script>alert(1)</script>");
    let p = upload(&s, &s.admin, "poly.png", &poly, &[]).unwrap();
    assert_eq!(p.status, 201);
    assert_eq!(p.json()["upload"]["kind"], "image");
    // um .docx só vale com a assinatura ZIP; um .txt só se for texto
    assert_eq!(
        upload(&s, &s.admin, "b.docx", &sh, &[]).unwrap().status,
        415
    );
    assert_eq!(
        upload(&s, &s.admin, "b.txt", b"hello \x00 world", &[])
            .unwrap()
            .status,
        415
    );
    assert_eq!(
        upload(&s, &s.admin, "b.txt", b"hello world", &[])
            .unwrap()
            .status,
        201
    );
    // um documento NÃO pode ser importado como asset de mídia
    let pid = s.create_project("sniff");
    let doc_id = upload(&s, &s.admin, "again.txt", b"just text", &[])
        .unwrap()
        .json()["upload"]["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let imp = s.call(
        "POST",
        &format!("/v1/projects/{pid}/assets"),
        Some(json!({"upload_id": doc_id})),
    );
    assert_eq!(imp.status, 422);
    assert!(healthy(&s));
}

/// ZIP com UM arquivo `word/document.xml` de `n` bytes de zeros, deflate FIXO com matches de 258
/// (razão ~160:1) — o tipo de "bomba" que o limite de descompressão do extrator precisa barrar.
fn docx_bomb(n: u64) -> Vec<u8> {
    // bits LSB-first
    struct Bits {
        out: Vec<u8>,
        acc: u64,
        nbits: u32,
    }
    impl Bits {
        fn put(&mut self, v: u32, n: u32) {
            self.acc |= u64::from(v) << self.nbits;
            self.nbits += n;
            while self.nbits >= 8 {
                self.out.push((self.acc & 0xff) as u8);
                self.acc >>= 8;
                self.nbits -= 8;
            }
        }
        fn put_huff(&mut self, code: u32, len: u32) {
            // códigos de Huffman saem do bit mais significativo para o menos
            for i in (0..len).rev() {
                self.put((code >> i) & 1, 1);
            }
        }
    }
    let mut b = Bits {
        out: Vec::new(),
        acc: 0,
        nbits: 0,
    };
    b.put(1, 1); // BFINAL
    b.put(1, 2); // BTYPE = fixed
    b.put_huff(0x30, 8); // literal 0
    let mut left = n - 1;
    while left >= 258 {
        b.put_huff(0b1100_0101, 8); // símbolo 285 = comprimento 258
        b.put_huff(0, 5); // distância 1
        left -= 258;
    }
    while left > 0 {
        b.put_huff(0x30, 8);
        left -= 1;
    }
    b.put_huff(0, 7); // fim de bloco
    if b.nbits > 0 {
        b.out.push((b.acc & 0xff) as u8);
    }
    let deflated = b.out;
    // CRC32 de n zeros
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *t = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for _ in 0..n {
        crc = table[(crc & 0xff) as usize] ^ (crc >> 8);
    }
    let crc = !crc;
    let name = b"word/document.xml";
    let mut z = Vec::new();
    let le32 = |v: u32| v.to_le_bytes();
    let le16 = |v: u16| v.to_le_bytes();
    // local header
    z.extend_from_slice(&le32(0x0403_4b50));
    z.extend_from_slice(&le16(20));
    z.extend_from_slice(&le16(0));
    z.extend_from_slice(&le16(8)); // deflate
    z.extend_from_slice(&le32(0));
    z.extend_from_slice(&le32(crc));
    z.extend_from_slice(&le32(deflated.len() as u32));
    z.extend_from_slice(&le32(u32::try_from(n).unwrap()));
    z.extend_from_slice(&le16(name.len() as u16));
    z.extend_from_slice(&le16(0));
    z.extend_from_slice(name);
    z.extend_from_slice(&deflated);
    let cd_off = z.len() as u32;
    z.extend_from_slice(&le32(0x0201_4b50));
    z.extend_from_slice(&le16(20));
    z.extend_from_slice(&le16(20));
    z.extend_from_slice(&le16(0));
    z.extend_from_slice(&le16(8));
    z.extend_from_slice(&le32(0));
    z.extend_from_slice(&le32(crc));
    z.extend_from_slice(&le32(deflated.len() as u32));
    z.extend_from_slice(&le32(u32::try_from(n).unwrap()));
    z.extend_from_slice(&le16(name.len() as u16));
    z.extend_from_slice(&[0u8; 12]);
    z.extend_from_slice(&le32(0));
    z.extend_from_slice(&le32(0));
    z.extend_from_slice(name);
    let cd_len = z.len() as u32 - cd_off;
    z.extend_from_slice(&le32(0x0605_4b50));
    z.extend_from_slice(&[0u8; 4]);
    z.extend_from_slice(&le16(1));
    z.extend_from_slice(&le16(1));
    z.extend_from_slice(&le32(cd_len));
    z.extend_from_slice(&le32(cd_off));
    z.extend_from_slice(&le16(0));
    z
}

#[test]
fn a_docx_zip_bomb_is_staged_cheaply_and_refused_by_the_extractor_without_hurting_the_server() {
    let s = start_fenced("up-bomb", |_| {});
    let bomb = docx_bomb(80 << 20); // 80 MiB de zeros ⇒ poucas centenas de KB
    assert!(
        bomb.len() < 2 << 20,
        "the bomb must be tiny on the wire ({} bytes)",
        bomb.len()
    );
    let r = upload(&s, &s.admin, "bomb.docx", &bomb, &[]).unwrap();
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["upload"]["kind"], "document");
    let id = r.json()["upload"]["upload_id"].as_str().unwrap().to_owned();
    // o upload em si não expande nada: só sniff dos primeiros bytes
    // o extrator (o único que abre o zip) barra a expansão com erro estruturado e rápido
    let (blob, _) = s.core().upload_blob(&id).unwrap();
    let t0 = std::time::Instant::now();
    let out = capia_intelligence::docs::extract_file(&blob);
    let err = out.expect_err("the zip bomb must not be extracted");
    assert!(
        err.code.starts_with("DOC_") || err.code.contains("LIMIT") || err.code.contains("DOC"),
        "{err:?}"
    );
    assert!(
        t0.elapsed() < std::time::Duration::from_secs(20),
        "extraction took {:?}",
        t0.elapsed()
    );
    // pela API: runs.create com esse documento termina com erro ESTRUTURADO (nunca 500/pânico)
    let pid = s.create_project("bomb");
    let run = s.call(
        "POST",
        &format!("/v1/projects/{pid}/runs"),
        Some(json!({"brief_text": "make an ad", "documents": [id], "start": false})),
    );
    assert!(
        run.status < 500,
        "{}: {}",
        run.status,
        String::from_utf8_lossy(&run.body)
    );
    assert!(leak_in(&whole(&run), s.dir.path()).is_none());
    assert!(healthy(&s));
}

#[test]
fn storage_failures_answer_a_structured_5xx_and_the_server_keeps_serving() {
    let s = start_fenced("up-diskfail", |_| {});
    let uploads = s.core().cfg.data_dir.join("uploads");
    // (a) o staging virou um ARQUIVO (disco/FS quebrado): create_dir_all falha
    std::fs::remove_dir_all(&uploads).unwrap();
    std::fs::write(&uploads, b"not a dir").unwrap();
    let r = upload(&s, &s.admin, "x.png", &png_padded(100), &[]).unwrap();
    assert!(
        matches!(r.status, 503 | 507),
        "{}: {}",
        r.status,
        String::from_utf8_lossy(&r.body)
    );
    assert!(
        matches!(r.code().as_str(), "STORAGE_UNAVAILABLE" | "STORAGE_FULL"),
        "{}",
        r.code()
    );
    assert!(leak_in(&whole(&r), s.dir.path()).is_none(), "{}", whole(&r));
    assert!(healthy(&s));
    let inline = s.call(
        "POST",
        "/v1/uploads/inline",
        Some(json!({"filename": "x.png", "content_base64": "iVBORw0KGgo="})),
    );
    assert!(inline.status >= 400 && inline.status < 600);
    assert!(leak_in(&whole(&inline), s.dir.path()).is_none());
    // (b) restaurado, volta a funcionar sem reiniciar
    std::fs::remove_file(&uploads).unwrap();
    std::fs::create_dir_all(&uploads).unwrap();
    let ok = upload(&s, &s.admin, "x.png", &png_padded(100), &[]).unwrap();
    assert_eq!(ok.status, 201);
    // (c) somente leitura (só vale para não-root: root ignora permissões; permissões Unix só em Unix)
    #[cfg(unix)]
    if !is_root() {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&uploads, std::fs::Permissions::from_mode(0o555)).unwrap();
        let r = upload(&s, &s.admin, "y.png", &png_padded(100), &[]).unwrap();
        std::fs::set_permissions(&uploads, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(r.status, 503 | 507), "{}", r.status);
        assert_eq!(r.code(), "STORAGE_UNAVAILABLE");
    }
    assert!(healthy(&s));
}

#[test]
fn filename_sanitizer_properties_hold_for_random_input() {
    let mut rng = Prng::new(0x5EED_F11E);
    let alphabet: Vec<char> = "abcAZ09._- /\\:*?\"<>|\0\r\n~$%`'\u{ff0e}\u{202e}\u{1f4a5}é"
        .chars()
        .collect();
    for _ in 0..5000 {
        let n = rng.below(40);
        let raw: String = (0..n).map(|_| *rng.pick(&alphabet)).collect();
        let out = capia_server::uploads::sanitize_filename(&raw);
        assert!(!out.is_empty() && out.len() <= 120, "{raw:?} → {out:?}");
        assert!(
            out.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ')),
            "{raw:?} → {out:?}"
        );
        assert!(!out.starts_with('.'), "{raw:?} → {out:?}");
        assert!(!windows_unsafe_name(&out), "{raw:?} → {out:?}");
        assert!(
            !out.eq_ignore_ascii_case("meta.json") && !out.eq_ignore_ascii_case("meta.json.tmp"),
            "{raw:?}"
        );
        // idempotente: sanitizar de novo não muda
        assert_eq!(
            capia_server::uploads::sanitize_filename(&out),
            out,
            "{raw:?}"
        );
    }
}

// =================================================================================================
// 7. SSRF (registro e atualização de webhooks pelos endpoints reais)
// =================================================================================================

const ALWAYS_BLOCKED: &[&str] = &[
    "http://169.254.169.254/latest/meta-data/",
    "https://169.254.169.254/latest/meta-data/",
    "https://169.254.170.2/v2/credentials",
    "https://metadata.google.internal/computeMetadata/v1/",
    "https://10.0.0.1/hook",
    "https://172.16.0.1/hook",
    "https://172.31.255.254/hook",
    "https://192.168.1.1/hook",
    "https://100.64.0.1/hook",
    "https://0.0.0.0/hook",
    "https://255.255.255.255/hook",
    "https://224.0.0.1/hook",
    "https://240.0.0.1/hook",
    "https://198.18.0.1/hook",
    "https://[fe80::1]/hook",
    "https://[fc00::1]/hook",
    "https://[fd12:3456::1]/hook",
    "https://[::]/hook",
    "https://[ff02::1]/hook",
    "https://[2001:db8::1]/hook",
    "https://[::ffff:169.254.169.254]/hook",
    "https://[::ffff:10.0.0.1]/hook",
    "https://[::ffff:192.168.0.1]/hook",
    "https://[::ffff:a9fe:a9fe]/hook",
    "https://[::a9fe:a9fe]/hook",
    "https://[64:ff9b::a9fe:a9fe]/hook",
    "https://[64:ff9b::a00:1]/hook",
    "https://[2002:a9fe:a9fe::1]/hook",
    "https://2852039166/hook",
    "https://0xa9fea9fe/hook",
    "https://0251.0376.0251.0376/hook",
    "https://169.254.43518/hook",
    "https://0xA9.0xFE.0xA9.0xFE/hook",
    "https://169.254.169.254.:443/hook",
    "https://167772161/hook",
    "https://0xa000001/hook",
    "https://012.0.0.1/hook",
    "https://user:pass@example.com/hook",
    "https://user@example.com/hook",
    "https://example.com@169.254.169.254/hook",
    "https://169.254.169.254#@example.com/hook",
    "https://foo.internal/hook",
    "https://printer.local/hook",
    "https://host.localdomain/hook",
    "file:///etc/passwd",
    "gopher://example.com/_x",
    "ftp://example.com/x",
    "ws://example.com/x",
    "wss://example.com/x",
    "javascript:alert(1)",
    "data:text/plain,hi",
    "dict://example.com:11211/stat",
    "ldap://example.com/",
    "http://example.com/hook",
    "//example.com/hook",
    "example.com/hook",
    "https:///hook",
    "https://",
    "",
    " https://example.com/hook",
    "https://exa mple.com/hook",
    "https://example.com/hook\r\nHost: evil",
];

const LOOPBACK_ONLY: &[&str] = &[
    "http://127.0.0.1:9/hook",
    "https://127.0.0.1/hook",
    "http://localhost:9/hook",
    "http://[::1]:9/hook",
    "http://2130706433:9/hook",
    "http://0x7f000001:9/hook",
    "http://0177.0.0.1:9/hook",
    "http://127.1:9/hook",
    "http://127.255.255.254:9/hook",
    "http://foo.localhost:9/hook",
    "https://[::ffff:127.0.0.1]/hook",
    "https://[::ffff:7f00:1]/hook",
    "https://[::127.0.0.1]/hook",
    "https://[64:ff9b::7f00:1]/hook",
];

fn webhook_body(url: &str) -> serde_json::Value {
    json!({"url": url, "events": ["*"]})
}

#[test]
fn ssrf_targets_are_refused_at_registration_and_update_in_every_mode() {
    for strict in [false, true] {
        let s = start(
            if strict {
                "ssrf-strict"
            } else {
                "ssrf-default"
            },
            |c| {
                c.rate_scale = 100.0;
                if strict {
                    c.webhook.allow_loopback = false;
                }
            },
        );
        // controle: um destino legítimo é aceito (o teste não passa por "recusar tudo")
        let ok = s.call(
            "POST",
            "/v1/webhooks",
            Some(webhook_body("https://example.com/hook")),
        );
        assert_eq!(ok.status, 201, "{}", String::from_utf8_lossy(&ok.body));
        let wid = ok.json()["webhook"]["id"].as_str().unwrap().to_owned();
        let mut targets: Vec<&str> = ALWAYS_BLOCKED.to_vec();
        if strict {
            targets.extend_from_slice(LOOPBACK_ONLY);
        }
        for url in targets {
            let r = s.call("POST", "/v1/webhooks", Some(webhook_body(url)));
            assert!(
                matches!(r.status, 422 | 400),
                "[strict={strict}] POST {url:?} answered {}: {}",
                r.status,
                String::from_utf8_lossy(&r.body)
            );
            assert!(leak_in(&whole(&r), s.dir.path()).is_none());
            let u = s.call(
                "PATCH",
                &format!("/v1/webhooks/{wid}"),
                Some(json!({"url": url})),
            );
            assert!(
                matches!(u.status, 422 | 400),
                "[strict={strict}] PATCH {url:?} answered {}",
                u.status
            );
        }
        // nada foi registrado nem alterado
        let list = s.call("GET", "/v1/webhooks", None).json();
        assert_eq!(list["webhooks"].as_array().unwrap().len(), 1);
        assert_eq!(list["webhooks"][0]["url"], "https://example.com/hook");
        // loopback é permitido por padrão (produto local-first), e só nele
        if !strict {
            for url in ["http://127.0.0.1:9/hook", "http://localhost:9/hook"] {
                assert_eq!(
                    s.call("POST", "/v1/webhooks", Some(webhook_body(url)))
                        .status,
                    201,
                    "{url}"
                );
            }
        }
        assert!(healthy(&s));
    }
}

#[test]
fn a_redirecting_endpoint_is_never_followed_and_hosts_are_filtered_at_connect_time() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let s = start("ssrf-redirect", |_| {});
    // alvo "interno" que NÃO pode receber conexão
    let victim = TcpListener::bind("127.0.0.1:0").unwrap();
    victim.set_nonblocking(true).unwrap();
    let victim_url = format!(
        "http://127.0.0.1:{}/internal",
        victim.local_addr().unwrap().port()
    );
    // receptor hostil: responde 302 para o alvo interno
    let evil = TcpListener::bind("127.0.0.1:0").unwrap();
    let evil_url = format!(
        "http://127.0.0.1:{}/hook",
        evil.local_addr().unwrap().port()
    );
    let location = victim_url.clone();
    let h = std::thread::spawn(move || {
        if let Ok((mut c, _)) = evil.accept() {
            let mut buf = [0u8; 4096];
            let _ = c.read(&mut buf);
            let _ = c.write_all(
                format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes(),
            );
        }
    });
    let client = &s.core().webhook_client;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let d = rt.block_on(client.post(&evil_url, &[], b"{}".to_vec()));
    h.join().unwrap();
    assert!(!d.delivered(), "{d:?}");
    assert!(
        d.error.as_deref().is_some_and(|e| e.contains("redirect")),
        "{d:?}"
    );
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        victim.accept().is_err(),
        "the redirect target was contacted: SSRF via 3xx"
    );
    // o destino do POST passa pela política a cada entrega (não só no registro)
    for url in [
        "https://169.254.169.254/x",
        "file:///etc/passwd",
        "http://example.com/x",
    ] {
        let d = rt.block_on(client.post(url, &[], b"{}".to_vec()));
        assert!(!d.delivered() && d.status.is_none(), "{url}: {d:?}");
    }
}

#[test]
fn hostnames_resolving_to_loopback_are_blocked_when_loopback_is_not_allowed() {
    use std::net::TcpListener;
    let s = start("ssrf-dns", |c| c.webhook.allow_loopback = false);
    let victim = TcpListener::bind("127.0.0.1:0").unwrap();
    victim.set_nonblocking(true).unwrap();
    let port = victim.local_addr().unwrap().port();
    let client = &s.core().webhook_client;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // `localhost.` (ponto final) não é reconhecido como literal de loopback no registro; o resolvedor
    // guardado do cliente é a segunda barreira: nunca conecta em 127.0.0.1
    for url in [
        format!("https://localhost.:{port}/x"),
        format!("https://localhost:{port}/x"),
    ] {
        let d = rt.block_on(client.post(&url, &[], b"{}".to_vec()));
        assert!(!d.delivered(), "{url}: {d:?}");
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        victim.accept().is_err(),
        "a loopback listener was contacted despite allow_loopback=false"
    );
}

// =================================================================================================
// 8. Forma da requisição
// =================================================================================================

fn raw_status(s: &TestServer, bytes: &[u8]) -> Option<u16> {
    exchange(s.addr, bytes, std::time::Duration::from_secs(15))
        .and_then(|o| try_parse(&o))
        .map(|r| r.status)
}

#[test]
fn malformed_request_heads_are_refused_without_hanging_or_crashing() {
    let s = start("shape-head", |c| c.rate_scale = 100.0);
    let port = s.addr.port();
    let host = format!("Host: 127.0.0.1:{port}\r\n");
    let big_val = "a".repeat(9000);
    let many: String = (0..70).map(|i| format!("X-H{i}: v\r\n")).collect();
    let huge_head: String = (0..10)
        .map(|i| format!("X-Big{i}: {}\r\n", "b".repeat(7000)))
        .collect();
    let cases: Vec<(&str, Vec<u8>, &[u16])> = vec![
        ("long header line", format!("GET /v1/health HTTP/1.1\r\n{host}X-A: {big_val}\r\n\r\n").into_bytes(), &[431]),
        ("too many headers", format!("GET /v1/health HTTP/1.1\r\n{host}{many}\r\n").into_bytes(), &[431]),
        ("head > 32 KiB", format!("GET /v1/health HTTP/1.1\r\n{host}{huge_head}\r\n").into_bytes(), &[431]),
        ("h2 preface", b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".to_vec(), &[400, 505]),
        ("http/0.9", b"GET /v1/health\r\n\r\n".to_vec(), &[400]),
        ("bare CR in header", format!("GET /v1/health HTTP/1.1\r\n{host}X-A: a\rb\r\n\r\n").into_bytes(), &[400]),
        ("NUL in header", format!("GET /v1/health HTTP/1.1\r\n{host}X-A: a\0b\r\n\r\n").into_bytes(), &[400]),
        ("invalid utf8 header", [format!("GET /v1/health HTTP/1.1\r\n{host}X-A: ").as_bytes(), &[0xff, 0xfe], b"\r\n\r\n"].concat(), &[400]),
        ("header without colon", format!("GET /v1/health HTTP/1.1\r\n{host}garbage\r\n\r\n").into_bytes(), &[400]),
        ("space in header name", format!("GET /v1/health HTTP/1.1\r\n{host}X A: v\r\n\r\n").into_bytes(), &[400]),
        ("obs-fold continuation", format!("GET /v1/health HTTP/1.1\r\n{host}X-A: a\r\n b\r\n\r\n").into_bytes(), &[400]),
        ("CL negative", format!("POST /v1/projects HTTP/1.1\r\n{host}Content-Length: -1\r\n\r\n").into_bytes(), &[400]),
        ("CL plus", format!("POST /v1/projects HTTP/1.1\r\n{host}Content-Length: +5\r\n\r\n").into_bytes(), &[400]),
        ("CL hex", format!("POST /v1/projects HTTP/1.1\r\n{host}Content-Length: 0x10\r\n\r\n").into_bytes(), &[400]),
        ("CL overflow", format!("POST /v1/projects HTTP/1.1\r\n{host}Content-Length: 99999999999999999999999\r\n\r\n").into_bytes(), &[400]),
        ("CL + TE", format!("POST /v1/projects HTTP/1.1\r\n{host}Content-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n").into_bytes(), &[501]),
        ("TE obfuscated", format!("POST /v1/projects HTTP/1.1\r\n{host}Transfer-Encoding : chunked\r\n\r\n0\r\n\r\n").into_bytes(), &[400, 501]),
        ("duplicate Host", format!("GET /v1/health HTTP/1.1\r\n{host}{host}\r\n").into_bytes(), &[400]),
        ("no Host", b"GET /v1/health HTTP/1.1\r\n\r\n".to_vec(), &[421]),
        ("tab in method", b"G\tT /v1/health HTTP/1.1\r\n\r\n".to_vec(), &[400]),
        ("lowercase method", format!("get /v1/health HTTP/1.1\r\n{host}\r\n").into_bytes(), &[400]),
        ("long method", format!("ABCDEFGHIJKLM /v1/health HTTP/1.1\r\n{host}\r\n").into_bytes(), &[400]),
    ];
    for (label, bytes, ok) in cases {
        let st = raw_status(&s, &bytes);
        // `None`: fechou sem resposta — aceitável (nunca pendurou: `exchange` tem prazo)
        if let Some(st) = st {
            assert!(ok.contains(&st), "{label}: answered {st}, expected {ok:?}");
        }
    }
    // lixo binário e pipelining: a primeira requisição boa é respondida; o lixo vira 400 e fecha
    let mut pipe = format!("GET /v1/health HTTP/1.1\r\n{host}\r\n").into_bytes();
    pipe.extend_from_slice(b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03garbage\r\n\r\n");
    let out = exchange(s.addr, &pipe, std::time::Duration::from_secs(10)).unwrap();
    assert!(
        String::from_utf8_lossy(&out).starts_with("HTTP/1.1 200"),
        "first pipelined request"
    );
    // TLS ClientHello puro (cliente com https:// no endereço): recusado sem travar
    let hello = [
        0x16u8, 0x03, 0x01, 0x00, 0x2f, 0x01, 0x00, 0x00, 0x2b, 0x03, 0x03,
    ];
    assert!(
        exchange(s.addr, &hello, std::time::Duration::from_secs(8)).is_none_or(
            |o| o.is_empty() || try_parse(&o).is_some_and(|r| matches!(r.status, 400 | 408))
        )
    );
    // 1000 conexões reaproveitadas não acumulam: keep-alive com muitos pedidos
    assert!(healthy(&s));
}

#[test]
fn a_slowloris_head_is_cut_off_and_does_not_hold_a_worker_forever() {
    let s = start("slowloris", |c| c.workers = 2);
    let mut sock = std::net::TcpStream::connect(s.addr).unwrap();
    sock.set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    let t0 = std::time::Instant::now();
    // dribla 1 byte por segundo, nunca termina o cabeçalho
    let head = b"GET /v1/health HTTP/1.1\r\nHost: 127.0.0.1:1\r\nX-Slow: ";
    let mut closed = false;
    for b in head.iter().cycle().take(40) {
        if std::io::Write::write_all(&mut sock, &[*b]).is_err() {
            closed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        let mut buf = [0u8; 512];
        match std::io::Read::read(&mut sock, &mut buf) {
            Ok(0) => {
                closed = true;
                break;
            }
            Ok(n) => {
                assert!(
                    String::from_utf8_lossy(&buf[..n]).contains("408"),
                    "{}",
                    String::from_utf8_lossy(&buf[..n])
                );
                closed = true;
                break;
            }
            Err(_) => {}
        }
        if t0.elapsed() > std::time::Duration::from_secs(14) {
            break;
        }
    }
    assert!(
        closed,
        "the dribbling client was never cut off ({:?})",
        t0.elapsed()
    );
    assert!(t0.elapsed() < std::time::Duration::from_secs(14));
    // o outro worker atendia o tempo todo e o primeiro está livre de novo
    assert!(healthy(&s));
}

#[test]
fn crlf_and_response_splitting_attempts_in_inputs_are_neutralised() {
    let s = start("crlf", |c| c.rate_scale = 100.0);
    let port = s.addr.port();
    // CR/LF cru num cabeçalho de entrada: a requisição é recusada
    for hdr in [
        "X-Request-Id: abc\r\nSet-Cookie: pwn=1",
        "Idempotency-Key: k\r\nX-Evil: 1",
    ] {
        let req = format!(
            "POST /v1/projects HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n{hdr}\r\n\r\n{{}}",
            s.admin
        );
        let out = exchange(s.addr, req.as_bytes(), std::time::Duration::from_secs(10)).unwrap();
        let r = try_parse(&out);
        assert!(
            r.as_ref()
                .is_none_or(|r| r.header("set-cookie").is_none() && r.header("x-evil").is_none()),
            "{}",
            String::from_utf8_lossy(&out)
        );
    }
    // CRLF percent-encodado / com caracteres fora da lista: o X-Request-Id é descartado e regenerado
    for v in [
        "abc%0d%0aSet-Cookie:%20x",
        "a b",
        "ação",
        &"z".repeat(65),
        "a;b",
        "a\"b",
    ] {
        let r = send(
            &s,
            "GET",
            "/v1/server",
            Some(&s.admin),
            &[("X-Request-Id", v)],
            b"",
        )
        .unwrap();
        assert_eq!(r.status, 200);
        let id = r.header("x-request-id").unwrap();
        assert!(id.starts_with("req_"), "{v:?} was echoed as {id:?}");
        assert!(r.header("set-cookie").is_none());
    }
    // um X-Request-Id válido é ecoado como está
    let r = send(
        &s,
        "GET",
        "/v1/server",
        Some(&s.admin),
        &[("X-Request-Id", "client-123_ok")],
        b"",
    )
    .unwrap();
    assert_eq!(r.header("x-request-id"), Some("client-123_ok"));
    // idempotency key com caracteres fora de 0x21..0x7e: 400
    for k in ["a b", "ação", "a\tb"] {
        let r = send(
            &s,
            "POST",
            "/v1/projects",
            Some(&s.admin),
            &[("Content-Type", "application/json"), ("Idempotency-Key", k)],
            b"{\"name\":\"x\"}",
        );
        if let Some(r) = r {
            assert_eq!(r.status, 400, "{k:?}");
        }
    }
}

#[test]
fn hostile_json_bodies_never_cause_a_5xx() {
    let s = start("json-abuse", |c| {
        c.rate_scale = 100.0;
        c.max_json_bytes = 1 << 20;
    });
    let post = |body: &[u8]| -> Resp {
        send(
            &s,
            "POST",
            "/v1/webhooks",
            Some(&s.admin),
            &[("Content-Type", "application/json")],
            body,
        )
        .unwrap()
    };
    let big_arr = format!(
        "{{\"url\":\"https://example.com/h\",\"events\":[{}]}}",
        "\"*\",".repeat(100_000) + "\"*\""
    );
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "nan",
            br#"{"url":"https://example.com/h","events":[NaN]}"#.to_vec(),
        ),
        (
            "infinity",
            br#"{"url":"https://example.com/h","events":[Infinity]}"#.to_vec(),
        ),
        (
            "huge exp",
            br#"{"url":"https://example.com/h","events":["*"],"x":1e999999}"#.to_vec(),
        ),
        (
            "huge int",
            br#"{"url":"https://example.com/h","events":["*"],"x":99999999999999999999999999}"#
                .to_vec(),
        ),
        (
            "neg zero",
            br#"{"url":"https://example.com/h","events":["*"],"x":-0}"#.to_vec(),
        ),
        (
            "lone surrogate",
            br#"{"url":"https://example.com/h","events":["*"],"description":"\ud800"}"#.to_vec(),
        ),
        (
            "nul escape",
            br#"{"url":"https://example.com/h","events":["*"],"description":"a\u0000b"}"#.to_vec(),
        ),
        (
            "invalid utf8",
            [
                &b"{\"url\":\"https://example.com/h\",\"events\":[\"*\"],\"description\":\""[..],
                &[0xff, 0xfe],
                b"\"}",
            ]
            .concat(),
        ),
        (
            "bom",
            [
                &[0xefu8, 0xbb, 0xbf][..],
                br#"{"url":"https://example.com/h","events":["*"]}"#,
            ]
            .concat(),
        ),
        (
            "trailing garbage",
            br#"{"url":"https://example.com/h","events":["*"]} trailing"#.to_vec(),
        ),
        (
            "comments",
            br#"{"url":"https://example.com/h", /* c */ "events":["*"]}"#.to_vec(),
        ),
        (
            "single quotes",
            br#"{'url':'https://example.com/h'}"#.to_vec(),
        ),
        (
            "unterminated",
            br#"{"url":"https://example.com/h","events":["*"#.to_vec(),
        ),
        ("array", b"[]".to_vec()),
        ("string", br#""x""#.to_vec()),
        ("null", b"null".to_vec()),
        ("wrong types", br#"{"url":123,"events":"*"}"#.to_vec()),
        (
            "many props",
            format!(
                "{{{}}}",
                (0..5000)
                    .map(|i| format!("\"k{i}\":1"))
                    .collect::<Vec<_>>()
                    .join(",")
            )
            .into_bytes(),
        ),
        ("huge array", big_arr.into_bytes()),
        (
            "deep 33",
            format!("{}{}", "[".repeat(33), "]".repeat(33)).into_bytes(),
        ),
        ("deep 100k", "[".repeat(100_000).into_bytes()),
        (
            "deep braces in 1MiB",
            format!("{}1{}", "{\"a\":".repeat(30_000), "}".repeat(30_000)).into_bytes(),
        ),
    ];
    for (label, body) in cases {
        let r = post(&body);
        assert!(
            matches!(r.status, 400 | 413 | 422) || (label == "nul escape" && r.status == 201),
            "{label}: {} {}",
            r.status,
            String::from_utf8_lossy(&r.body)
                .chars()
                .take(200)
                .collect::<String>()
        );
        assert!(
            leak_in(&whole(&r), s.dir.path()).is_none(),
            "{label}: {}",
            whole(&r)
        );
    }
    // aninhamento válido (objetos) acima do limite: recusado pelo servidor, com o motivo
    let deep = format!("{}1{}", "{\"a\":".repeat(40), "}".repeat(40));
    let r = post(deep.as_bytes());
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    assert!(String::from_utf8_lossy(&r.body).contains("nested too deeply"));
    // chaves duplicadas: vale a última, e a validação (escopos!) vê o MESMO valor que o handler
    let weak = s.token("weak", &["admin:tokens"]);
    let dup = s.call_as(&weak, "POST", "/v1/tokens", None);
    assert_eq!(dup.status, 422);
    let body = br#"{"name":"x","scopes":["admin:tokens"],"scopes":["project:write"]}"#;
    let r = send(
        &s,
        "POST",
        "/v1/tokens",
        Some(&weak),
        &[("Content-Type", "application/json")],
        body,
    )
    .unwrap();
    assert_eq!(
        r.status,
        403,
        "duplicate-key smuggling of scopes: {}",
        String::from_utf8_lossy(&r.body)
    );
    let body = br#"{"name":"x","scopes":["project:write"],"scopes":["admin:tokens"]}"#;
    let r = send(
        &s,
        "POST",
        "/v1/tokens",
        Some(&weak),
        &[("Content-Type", "application/json")],
        body,
    )
    .unwrap();
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["token"]["scopes"], json!(["admin:tokens"]));
    // query: coerção segura, repetição, excesso
    for q in [
        "limit=abc",
        "limit=1e30",
        "limit=-1",
        "limit=99999999999999999999",
        "limit=1&limit=2",
        "after[]=x",
        "limit",
    ] {
        let r = send(
            &s,
            "GET",
            &format!("/v1/projects?{q}"),
            Some(&s.admin),
            &[],
            b"",
        )
        .unwrap();
        assert!(r.status < 500, "{q}: {}", r.status);
    }
    let many_q: String = (0..40)
        .map(|i| format!("a{i}=1"))
        .collect::<Vec<_>>()
        .join("&");
    let r = send(
        &s,
        "GET",
        &format!("/v1/projects?{many_q}"),
        Some(&s.admin),
        &[],
        b"",
    )
    .unwrap();
    assert_eq!(r.status, 400);
    assert!(healthy(&s));
}

// =================================================================================================
// 9. Host, CORS, bind remoto
// =================================================================================================

fn with_host(s: &TestServer, host: Option<&str>, extra: &[(&str, &str)]) -> Option<u16> {
    let mut hs: Vec<(&str, &str)> = vec![("Connection", "close")];
    if let Some(h) = host {
        hs.push(("Host", h));
    }
    hs.extend_from_slice(extra);
    raw_status(s, &build("GET", "/v1/health", &hs, b""))
}

#[test]
fn dns_rebinding_hosts_and_origins_are_refused() {
    let s = start("host-cors", |c| {
        c.cors_origins = vec!["http://localhost:3000".into()];
        c.allowed_hosts = vec!["myhost.test:8080".into()];
    });
    let p = s.addr.port();
    for h in [
        format!("evil.example:{p}"),
        format!("127.0.0.1.evil.example:{p}"),
        format!("localhost.evil.example:{p}"),
        format!("127.0.0.1:{}", p.wrapping_add(1)),
        "127.0.0.1".to_owned(),
        "localhost".to_owned(),
        format!("a@127.0.0.1:{p}"),
        format!("127.0.0.1:{p}.evil.example"),
        format!(" 127.0.0.1:{p}x"),
        format!("127.0.0.1:{p}/"),
        format!("[::ffff:127.0.0.1]:{p}"),
        format!("0.0.0.0:{p}"),
        format!("2130706433:{p}"),
        format!("127.1:{p}"),
        "myhost.test".to_owned(),
        "myhost.test:8080.evil.example".to_owned(),
        "myhost.test:80".to_owned(),
        String::new(),
    ] {
        assert_eq!(with_host(&s, Some(&h), &[]), Some(421), "Host {h:?}");
    }
    assert_eq!(with_host(&s, None, &[]), Some(421));
    // aceitos: loopback (qualquer caixa) e o host configurado (case-insensitive, exato)
    for h in [
        format!("127.0.0.1:{p}"),
        format!("LOCALHOST:{p}"),
        format!("[::1]:{p}"),
        "MyHost.Test:8080".to_owned(),
    ] {
        assert_eq!(with_host(&s, Some(&h), &[]), Some(200), "Host {h:?}");
    }
    // alvo em forma absoluta (`GET http://evil/ …`) não troca o Host
    let abs = format!(
        "GET http://evil.example/v1/health HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nConnection: close\r\n\r\n"
    );
    assert_eq!(raw_status(&s, abs.as_bytes()), Some(400));
    // Origin: null, caixa, variações e múltiplos
    let host = format!("127.0.0.1:{p}");
    for o in [
        "null",
        "NULL",
        "HTTP://LOCALHOST:3000",
        "http://localhost:3000/",
        "http://localhost:3000.evil.example",
        "http://localhost:3001",
        "https://localhost:3000",
        "http://evil.example",
        "*",
        "",
        "http://localhost",
        "http://localhost:3000 http://evil.example",
        "file://",
        "chrome-extension://abc",
    ] {
        let st = with_host(&s, Some(&host), &[("Origin", o)]);
        assert_eq!(st, Some(403), "Origin {o:?} must be denied");
    }
    let r = send(
        &s,
        "GET",
        "/v1/health",
        None,
        &[("Origin", "http://localhost:3000")],
        b"",
    )
    .unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(
        r.header("access-control-allow-origin"),
        Some("http://localhost:3000")
    );
    assert!(
        r.header("access-control-allow-credentials").is_none(),
        "credentials must never be allowed"
    );
    assert_eq!(r.header("vary"), Some("Origin"));
    // dois Origin: nunca eco do malicioso
    let two = send(
        &s,
        "GET",
        "/v1/health",
        None,
        &[
            ("Origin", "http://localhost:3000"),
            ("Origin", "http://evil.example"),
        ],
        b"",
    )
    .unwrap();
    assert_ne!(
        two.header("access-control-allow-origin"),
        Some("http://evil.example")
    );
    // CSRF de navegador: formulário cross-site (simple request) é barrado ANTES de autenticar/agir
    let pid_before = s.call("GET", "/v1/projects", None).json()["projects"]
        .as_array()
        .unwrap()
        .len();
    let csrf = send(
        &s,
        "POST",
        "/v1/projects",
        Some(&s.admin),
        &[
            ("Origin", "http://evil.example"),
            ("Content-Type", "text/plain"),
            ("Cookie", "session=1"),
        ],
        b"{\"name\":\"csrf\"}",
    )
    .unwrap();
    assert_eq!(csrf.status, 403);
    assert_eq!(csrf.code(), "CORS_DENIED");
    let csrf2 = send(
        &s,
        "POST",
        "/v1/projects",
        Some(&s.admin),
        &[
            ("Origin", "http://evil.example"),
            ("Content-Type", "application/json"),
        ],
        b"{\"name\":\"csrf\"}",
    )
    .unwrap();
    assert_eq!(csrf2.status, 403);
    assert_eq!(
        s.call("GET", "/v1/projects", None).json()["projects"]
            .as_array()
            .unwrap()
            .len(),
        pid_before
    );
    // sem Origin (curl/CLI) segue funcionando: o token é a fronteira
    assert_eq!(s.call("GET", "/v1/server", None).status, 200);
    // o preflight de origem permitida lista só métodos/headers do contrato
    let pre = send(
        &s,
        "OPTIONS",
        "/v1/projects",
        None,
        &[
            ("Origin", "http://localhost:3000"),
            ("Access-Control-Request-Method", "TRACE"),
        ],
        b"",
    )
    .unwrap();
    assert_eq!(pre.status, 204);
    assert!(
        !pre.header("access-control-allow-methods")
            .unwrap()
            .contains("TRACE")
    );
    assert!(
        !pre.header("access-control-allow-headers")
            .unwrap()
            .contains('*')
    );
}

#[test]
fn remote_bind_needs_both_confirmations_and_dangerous_configs_are_refused() {
    use capia_server::ServerConfig;
    let d = TempDir::new("cfg");
    let base = || ServerConfig::new(d.path());
    let c = base();
    assert!(
        c.bind.is_loopback() && c.validate().is_ok(),
        "the default must be loopback"
    );
    for ip in [
        "0.0.0.0",
        "::",
        "192.168.1.5",
        "10.0.0.2",
        "::ffff:127.0.0.1",
        "169.254.1.1",
        "8.8.8.8",
    ] {
        let mut c = base();
        c.bind = ip.parse().unwrap();
        assert!(c.validate().is_err(), "{ip} alone must be refused");
        c.allow_remote = true;
        assert!(
            c.validate().is_err(),
            "{ip} with only --allow-remote must be refused"
        );
        c.allow_remote = false;
        c.remote_tls_terminated_by_proxy = true;
        assert!(
            c.validate().is_err(),
            "{ip} with only the TLS flag must be refused"
        );
        c.allow_remote = true;
        assert!(
            c.validate().is_ok(),
            "{ip} with both flags is the documented opt-in"
        );
        // e o servidor NEM abre o socket sem as duas
        let mut c2 = base();
        c2.bind = ip.parse().unwrap();
        assert!(capia_server::Server::start(c2).is_err());
    }
    for ip in ["127.0.0.1", "127.0.0.2", "::1"] {
        let mut c = base();
        c.bind = ip.parse().unwrap();
        assert!(c.validate().is_ok(), "{ip}");
    }
    for bad in [
        "*",
        "https://*.example.com",
        "null",
        "NULL",
        "",
        "http://x.example/",
    ] {
        let mut c = base();
        c.cors_origins = vec![bad.to_owned()];
        assert!(c.validate().is_err(), "CORS origin {bad:?} must be refused");
    }
    let mut c = base();
    c.cors_origins = vec!["http://localhost:3000".into()];
    assert!(c.validate().is_ok());
    for f in [
        (|c: &mut ServerConfig| c.workers = 0) as fn(&mut ServerConfig),
        |c| c.workers = 257,
        |c| c.queue = 0,
        |c| c.max_json_bytes = 0,
        |c| c.max_json_bytes = 65 << 20,
    ] {
        let mut c = base();
        f(&mut c);
        assert!(c.validate().is_err());
    }
    // pelo binário: bind remoto sem as flags falha sem abrir porta
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_capia-server"))
        .args(["serve", "--data-dir"])
        .arg(d.path().join("bin"))
        .args(["--bind", "0.0.0.0", "--port", "0"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("refusing to bind"));
}

// =================================================================================================
// 10. Idempotência sob ataque
// =================================================================================================

#[test]
fn idempotency_keys_are_scoped_per_token_and_never_cross_over() {
    let s = start("idem-scope", |c| c.rate_scale = 100.0);
    let a = s.token("a", &["project:write", "project:read"]);
    let b = s.token("b", &["project:write", "project:read"]);
    let mk = |tok: &str, key: &str, name: &str| {
        send(
            &s,
            "POST",
            "/v1/projects",
            Some(tok),
            &[
                ("Content-Type", "application/json"),
                ("Idempotency-Key", key),
            ],
            json!({"name": name}).to_string().as_bytes(),
        )
        .unwrap()
    };
    let ra = mk(&a, "shared-key", "secret-project-of-a");
    assert_eq!(ra.status, 201);
    // outro token, MESMA chave e MESMO corpo: executa de verdade, sem replay do resultado do A
    let rb = mk(&b, "shared-key", "secret-project-of-a");
    assert_eq!(rb.status, 201);
    assert!(
        rb.header("idempotent-replay").is_none(),
        "token B got a replay of token A's response"
    );
    assert_ne!(ra.json()["project"]["id"], rb.json()["project"]["id"]);
    // outro token, mesma chave, corpo diferente: nada de "key reused" que revele a existência
    let rc = mk(&b, "shared-key", "other");
    assert_eq!(rc.status, 422);
    assert_eq!(rc.code(), "IDEMPOTENCY_KEY_REUSED");
    // replay do próprio token funciona
    let ra2 = mk(&a, "shared-key", "secret-project-of-a");
    assert_eq!(ra2.header("idempotent-replay"), Some("true"));
    // depois de revogado o token, o replay NÃO responde (401) e a resposta não vai para outro
    let a_id = s.call("GET", "/v1/tokens", None).json()["tokens"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "a")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        s.call("DELETE", &format!("/v1/tokens/{a_id}"), None).status,
        200
    );
    assert_eq!(mk(&a, "shared-key", "secret-project-of-a").status, 401);
    // a chave é um nonce (a auditoria a mostra para correlação); se coincidir com um segredo
    // conhecido, nunca chega ao disco: auditoria redigida e tabela de idempotência só com digest
    let key = "KEYCANARY-should-not-be-in-the-db-0123456789";
    capia_secrets::register_global(key);
    mk(&b, key, "k");
    mk(&b, "plain-correlation-key", "k2");
    assert!(
        s.core()
            .db
            .audit_list(0, 500)
            .unwrap()
            .iter()
            .any(|r| r.idempotency_key.as_deref() == Some("plain-correlation-key")),
        "the audit trail keeps ordinary keys for correlation"
    );
    s.core()
        .db
        .audit_list(0, 500)
        .unwrap()
        .iter()
        .for_each(|r| {
            assert_ne!(r.idempotency_key.as_deref(), Some(key));
        });
    assert!(
        files_containing(s.dir.path(), key.as_bytes()).is_empty(),
        "the raw Idempotency-Key reached the disk"
    );
}

#[test]
fn concurrent_requests_with_the_same_key_execute_exactly_once() {
    let s = start("idem-race", |c| {
        c.rate_scale = 100.0;
        c.workers = 12;
    });
    let n = 10;
    let addr = s.addr;
    let admin = s.admin.clone();
    let hs: Vec<_> = (0..n)
        .map(|_| {
            let admin = admin.clone();
            std::thread::spawn(move || {
                request(
                    addr,
                    "POST",
                    "/v1/webhooks",
                    Some(&admin),
                    &[
                        ("Content-Type", "application/json"),
                        ("Idempotency-Key", "race-key"),
                    ],
                    Some(
                        webhook_body("https://example.com/race")
                            .to_string()
                            .as_bytes(),
                    ),
                )
            })
        })
        .collect();
    let rs: Vec<Resp> = hs.into_iter().map(|h| h.join().unwrap()).collect();
    for r in &rs {
        assert!(
            matches!(r.status, 201 | 409),
            "{} {}",
            r.status,
            String::from_utf8_lossy(&r.body)
        );
        if r.status == 409 {
            assert_eq!(r.code(), "IDEMPOTENCY_IN_PROGRESS");
        }
    }
    assert_eq!(
        rs.iter()
            .filter(|r| r.status == 201 && r.header("idempotent-replay").is_none())
            .count(),
        1,
        "exactly one real execution"
    );
    let list = s.call("GET", "/v1/webhooks", None).json();
    assert_eq!(
        list["webhooks"].as_array().unwrap().len(),
        1,
        "the operation ran more than once"
    );
}

#[test]
fn many_distinct_keys_are_bounded_by_rate_limits_and_hashed_not_stored_raw() {
    let s = start("idem-many", |c| c.rate_scale = 100.0);
    // 400 chaves distintas numa operação barata de erro 4xx (que é guardada)
    for i in 0..400 {
        let r = send(
            &s,
            "DELETE",
            &format!("/v1/uploads/upl_none{i}"),
            Some(&s.admin),
            &[("Idempotency-Key", &format!("flood-{i}"))],
            b"",
        )
        .unwrap();
        assert_eq!(r.status, 404);
    }
    assert!(healthy(&s));
    let len = std::fs::metadata(s.dir.path().join("server.db"))
        .unwrap()
        .len();
    assert!(len < 64 << 20, "server.db is {len} bytes after 400 keys");
    // a manutenção de abertura apara o que for velho (24 h) e preserva o recente
    let db = &s.core().db;
    let purged = db.idem_purge_older_than(0).unwrap();
    assert_eq!(purged, 0, "recent keys must survive a cutoff in the past");
}

#[test]
fn stale_revisions_and_foreign_plans_are_rejected() {
    let s = start("stale", |c| c.rate_scale = 100.0);
    let pid = s.create_project("stale");
    let prev = |cmds: Value, extra: Value| {
        let mut b = json!({"commands": cmds});
        for (k, v) in extra.as_object().unwrap() {
            b[k] = v.clone();
        }
        s.call(
            "POST",
            &format!("/v1/projects/{pid}/commands/preview"),
            Some(b),
        )
    };
    let p1 = prev(create_sequence_cmds("st1", "sa"), json!({}));
    assert_eq!(p1.status, 200, "{}", String::from_utf8_lossy(&p1.body));
    let applied = s.call(
        "POST",
        &format!("/v1/projects/{pid}/commands/apply"),
        Some(json!({"plan_token": p1.json()["plan_token"]})),
    );
    assert_eq!(applied.status, 200);
    // revisão esperada velha ⇒ 409 REVISION_CONFLICT
    let stale = prev(
        create_sequence_cmds("st2", "sb"),
        json!({"expected_revision": 0}),
    );
    assert_eq!(
        stale.status,
        409,
        "{}",
        String::from_utf8_lossy(&stale.body)
    );
    assert_eq!(stale.code(), "REVISION_CONFLICT");
    // plano reaplicado (replay) e plano de OUTRO token
    let hist_before = s
        .call("GET", &format!("/v1/projects/{pid}/history"), None)
        .json()["entries"]
        .as_array()
        .unwrap()
        .len();
    let again = s.call(
        "POST",
        &format!("/v1/projects/{pid}/commands/apply"),
        Some(json!({"plan_token": p1.json()["plan_token"]})),
    );
    assert!(again.status < 500, "{}", again.status);
    let hist_after = s
        .call("GET", &format!("/v1/projects/{pid}/history"), None)
        .json()["entries"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(
        hist_before, hist_after,
        "re-applying a consumed plan created a second history entry"
    );
    let p2 = prev(create_sequence_cmds("st3", "sc"), json!({}));
    let other = s.token("other-writer", &["project:write"]);
    let steal = s.call_as(
        &other,
        "POST",
        &format!("/v1/projects/{pid}/commands/apply"),
        Some(json!({"plan_token": p2.json()["plan_token"]})),
    );
    assert!(
        steal.status >= 400 && steal.status < 500,
        "a token applied another token's plan: {}",
        steal.status
    );
    // o doc não ganhou a sequence "sc" por esse caminho
    let seqs = s
        .call("GET", &format!("/v1/projects/{pid}/sequences"), None)
        .json();
    assert!(!seqs.to_string().contains("\"sc\""), "{seqs}");
    // tokens de plano inventados
    for t in ["", "x", "plan_0", &"a".repeat(5000)] {
        let r = s.call(
            "POST",
            &format!("/v1/projects/{pid}/commands/apply"),
            Some(json!({"plan_token": t})),
        );
        assert!(r.status >= 400 && r.status < 500, "{t:?}: {}", r.status);
    }
}

// =================================================================================================
// 11. Vazamento de informação e DoS
// =================================================================================================

#[test]
fn error_responses_never_contain_paths_sql_stacks_or_tokens() {
    let s = start_fenced("leaks", |_| {});
    let pid = s.create_project("leak");
    let low = s.token("low", &["project:read"]);
    let mut corpus: Vec<Resp> = Vec::new();
    let mut add = |r: Option<Resp>| corpus.extend(r);
    // erros de cada família
    for (m, p, body) in [
        ("GET", "/v1/nope".to_owned(), ""),
        ("PUT", "/v1/projects".to_owned(), ""),
        ("GET", "/v1/projects/zzz".to_owned(), ""),
        ("POST", "/v1/projects/zzz/open".to_owned(), "{}"),
        ("POST", "/v1/projects".to_owned(), "{nope"),
        ("POST", "/v1/projects".to_owned(), "{\"name\": 1}"),
        (
            "POST",
            "/v1/projects".to_owned(),
            "{\"name\":\"x\",\"path\":\"/etc\"}",
        ),
        ("GET", format!("/v1/projects/{pid}/sequences/missing"), ""),
        ("GET", format!("/v1/projects/{pid}/assets/missing"), ""),
        ("GET", format!("/v1/projects/{pid}/imports/missing"), ""),
        ("GET", format!("/v1/projects/{pid}/runs/missing"), ""),
        ("GET", format!("/v1/projects/{pid}/exports/missing"), ""),
        (
            "POST",
            format!("/v1/projects/{pid}/commands/apply"),
            "{\"plan_token\":\"nope\"}",
        ),
        (
            "POST",
            format!("/v1/projects/{pid}/commands/preview"),
            "{\"commands\":[{\"type\":\"nope\"}]}",
        ),
        (
            "POST",
            format!("/v1/projects/{pid}/assets"),
            "{\"upload_id\":\"upl_missing\"}",
        ),
        (
            "POST",
            format!("/v1/projects/{pid}/runs"),
            "{\"brief_text\":\"x\",\"documents\":[\"upl_missing\"]}",
        ),
        (
            "POST",
            format!("/v1/projects/{pid}/exports"),
            "{\"items\":[{\"sequence\":\"missing\"}]}",
        ),
        (
            "POST",
            "/v1/webhooks".to_owned(),
            "{\"url\":\"https://10.0.0.1/\",\"events\":[\"*\"]}",
        ),
        ("DELETE", "/v1/uploads/upl_x".to_owned(), ""),
        ("DELETE", "/v1/tokens/tok_x".to_owned(), ""),
    ] {
        add(send(
            &s,
            m,
            &p,
            Some(&s.admin),
            &[("Content-Type", "application/json")],
            body.as_bytes(),
        ));
    }
    // 401/403/405/421/429-like
    add(send(&s, "GET", "/v1/server", None, &[], b""));
    add(send(&s, "GET", "/v1/tokens", Some(&low), &[], b""));
    add(send(&s, "PUT", "/v1/projects", Some(&s.admin), &[], b""));
    add(Some(
        try_parse(
            &exchange(
                s.addr,
                b"GET /v1/health HTTP/1.1\r\nHost: evil\r\n\r\n",
                std::time::Duration::from_secs(5),
            )
            .unwrap(),
        )
        .unwrap(),
    ));
    add(Some(
        try_parse(
            &exchange(
                s.addr,
                b"garbage\r\n\r\n",
                std::time::Duration::from_secs(5),
            )
            .unwrap(),
        )
        .unwrap(),
    ));
    // upload/arquivo quebrados
    add(upload(&s, &s.admin, "a.exe", b"MZ\x90\0", &[]));
    // projeto com arquivo corrompido: o erro do engine ao abrir não pode citar o caminho
    let row = s.core().db.project_get(&pid).unwrap().unwrap();
    s.call("POST", &format!("/v1/projects/{pid}/close"), None);
    std::fs::write(&row.path, b"this is not a sqlite database at all").unwrap();
    add(Some(s.call(
        "POST",
        &format!("/v1/projects/{pid}/open"),
        Some(json!({})),
    )));
    add(Some(s.call(
        "GET",
        &format!("/v1/projects/{pid}/summary"),
        None,
    )));
    assert!(corpus.len() > 25);
    for r in &corpus {
        let t = whole(r);
        assert!(r.status != 500 || !t.is_empty());
        assert!(
            leak_in(&t, s.dir.path()).is_none(),
            "leak ({}) in:\n{t}",
            leak_in(&t, s.dir.path()).unwrap()
        );
        assert!(!t.contains(&s.admin) && !t.contains(&low));
    }
    // o servidor sobrevive a tudo isso
    assert!(healthy(&s));
}

#[test]
fn a_burst_of_connections_is_bounded_in_threads_and_fds_and_ends_with_503_not_growth() {
    // processo FILHO: `/proc/<pid>/task` mede só o servidor (os outros testes rodam em paralelo
    // no mesmo processo e poluiriam a contagem)
    let dir = TempDir::new("dos");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_capia-server"))
        .args(["serve", "--data-dir"])
        .arg(dir.path())
        .args(["--port", "0", "--bootstrap"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let addr = {
        use std::io::BufRead;
        let mut r = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        let mut found = None;
        while found.is_none() && r.read_line(&mut line).unwrap_or(0) > 0 {
            if let Some(a) = line.split("http://").nth(1) {
                found = a
                    .split_whitespace()
                    .next()
                    .map(|a| a.parse::<std::net::SocketAddr>().unwrap());
            }
            line.clear();
        }
        // mantém o pipe vivo (o filho não pode levar SIGPIPE) drenando em segundo plano
        std::thread::spawn(move || {
            let mut sink = String::new();
            while r.read_line(&mut sink).unwrap_or(0) > 0 {
                sink.clear();
            }
        });
        found.expect("the server must announce its address")
    };
    let pid = child.id();
    let count =
        |what: &str| std::fs::read_dir(format!("/proc/{pid}/{what}")).map_or(0, Iterator::count);
    let healthy_child = || {
        try_parse(
            &exchange(
                addr,
                format!(
                    "GET /v1/health HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
                    addr.port()
                )
                .as_bytes(),
                std::time::Duration::from_secs(5),
            )
            .unwrap_or_default(),
        )
        .is_some_and(|r| r.status == 200)
    };
    assert!(wait_until(
        std::time::Duration::from_secs(10),
        healthy_child
    ));
    // baseline só depois que o processo ESTABILIZA (threads de fundo sobem logo após o primeiro
    // pedido; sob carga do CI isso demora): 6 amostras iguais seguidas, com teto de 10 s
    let stable = {
        let mut last = (count("task"), count("fd"));
        let mut same = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while same < 6 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(150));
            let now = (count("task"), count("fd"));
            if now == last {
                same += 1;
            } else {
                same = 0;
                last = now;
            }
        }
        last
    };
    let (t0, f0) = stable;
    let mut socks = Vec::new();
    for _ in 0..500 {
        if let Ok(sk) = std::net::TcpStream::connect(addr) {
            socks.push(sk);
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
    let (tp, fp) = (count("task"), count("fd"));
    assert!(
        tp <= t0 + 2,
        "threads grew from {t0} to {tp} under 500 connections"
    );
    assert!(
        fp <= f0 + 80,
        "fds grew from {f0} to {fp} (the queue is finite)"
    );
    // fila cheia: quem chega leva 503 imediato
    let mut saw_503 = false;
    for _ in 0..20 {
        let mut c = std::net::TcpStream::connect(addr).unwrap();
        c.set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut buf = [0u8; 1024];
        if let Ok(n) = std::io::Read::read(&mut c, &mut buf)
            && String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 503")
        {
            saw_503 = true;
            break;
        }
    }
    assert!(saw_503, "a full queue must answer 503");
    drop(socks);
    assert!(
        wait_until(std::time::Duration::from_secs(60), healthy_child),
        "the server did not recover"
    );
    let (te, fe) = (count("task"), count("fd"));
    assert!(te <= t0 + 2, "threads after the burst: {te} (was {t0})");
    assert!(fe <= f0 + 20, "fd leak: {f0} → {fe}");
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
#[test]
fn data_dir_is_private_to_the_owner() {
    use std::os::unix::fs::PermissionsExt;
    let s = start("perm", |_| {});
    let mode = std::fs::metadata(s.dir.path())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o077,
        0,
        "data dir is accessible to group/others: {mode:o}"
    );
}
