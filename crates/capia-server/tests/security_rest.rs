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
