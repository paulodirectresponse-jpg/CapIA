//! Webhooks: registro, assinatura, retry/backoff, dead-letter, reentrega, isolamento de endpoint
//! lento, redirect, duplicação, segredo indisponível, revalidação de URL e ataques do receptor.
#![allow(
    unreachable_pub,
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines
)]

mod common;
#[path = "common/receiver.rs"]
mod receiver;

use capia_server::webhooks::VerifyError;
use common::*;
use receiver::{Action, Receiver};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn wait_until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < timeout {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    f()
}

fn now_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Cria um webhook e devolve `(id, segredo)`.
fn hook(s: &TestServer, url: &str, events: &[&str]) -> (String, String) {
    let r = s.call(
        "POST",
        "/v1/webhooks",
        Some(json!({"url": url, "events": events})),
    );
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let b = r.json();
    (
        b["webhook"]["id"].as_str().unwrap().to_owned(),
        b["secret"].as_str().unwrap().to_owned(),
    )
}

fn send_test(s: &TestServer, id: &str) -> String {
    let r = s.call("POST", &format!("/v1/webhooks/{id}/test"), None);
    assert_eq!(r.status, 202, "{}", String::from_utf8_lossy(&r.body));
    r.json()["event_id"].as_str().unwrap().to_owned()
}

fn deliveries(s: &TestServer, id: &str) -> Vec<Value> {
    let r = s.call("GET", &format!("/v1/webhooks/{id}/deliveries"), None);
    assert_eq!(r.status, 200);
    r.json()["deliveries"].as_array().unwrap().clone()
}

fn first_state(s: &TestServer, id: &str) -> String {
    deliveries(s, id)
        .first()
        .map(|d| d["state"].as_str().unwrap().to_owned())
        .unwrap_or_default()
}

#[test]
fn registration_validates_urls_events_and_never_leaks_the_secret() {
    let s = start("wh-reg", |_| {});
    for url in [
        "http://169.254.169.254/latest/meta-data/",
        "http://10.0.0.5/hook",
        "http://192.168.1.10/hook",
        "http://172.16.0.1/hook",
        "http://0.0.0.0/hook",
        "http://[fe80::1]/hook",
        "http://[fd00::1]/hook",
        "http://metadata.google.internal/computeMetadata/v1/",
        "https://printer.local/hook",
        "http://example.com/hook",
        "ftp://example.com/hook",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "http://user:pw@127.0.0.1/hook",
        "not a url",
    ] {
        let r = s.call(
            "POST",
            "/v1/webhooks",
            Some(json!({"url": url, "events": ["*"]})),
        );
        assert_eq!(
            r.status,
            422,
            "{url} -> {}",
            String::from_utf8_lossy(&r.body)
        );
        assert_eq!(r.code(), "INVALID_PARAMS", "{url}");
    }
    // cada recusa vale também para o PATCH
    let rx = Receiver::start();
    let (id, secret) = hook(&s, &rx.url(), &["run.completed"]);
    let bad = s.call(
        "PATCH",
        &format!("/v1/webhooks/{id}"),
        Some(json!({"url": "http://169.254.169.254/"})),
    );
    assert_eq!(bad.status, 422);
    // evento desconhecido / lista vazia / url longa
    for body in [
        json!({"url": rx.url(), "events": ["nope.event"]}),
        json!({"url": rx.url(), "events": []}),
        json!({"url": format!("http://127.0.0.1:1/{}", "a".repeat(2100)), "events": ["*"]}),
        json!({"url": rx.url(), "events": ["*"], "extra": 1}),
    ] {
        assert_eq!(s.call("POST", "/v1/webhooks", Some(body)).status, 422);
    }
    // o segredo aparece uma única vez
    assert!(secret.starts_with("whsec_"));
    let get = s.call("GET", &format!("/v1/webhooks/{id}"), None);
    let list = s.call("GET", "/v1/webhooks", None);
    for r in [&get, &list] {
        assert_eq!(r.status, 200);
        assert!(!String::from_utf8_lossy(&r.body).contains(&secret));
    }
    assert_eq!(get.json()["webhook"]["secret_configured"], true);
    for entry in walk(s.dir.path()) {
        if let Ok(bytes) = std::fs::read(&entry) {
            assert!(
                !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
                "the signing secret leaked into {}",
                entry.display()
            );
        }
    }
    // escopo: só `webhook:manage`
    let reader = s.token("reader", &["project:read", "run:read"]);
    assert_eq!(s.call_as(&reader, "GET", "/v1/webhooks", None).status, 403);
    assert_eq!(
        s.call_as(&reader, "POST", &format!("/v1/webhooks/{id}/test"), None)
            .status,
        403
    );
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
fn delivery_is_signed_and_the_receiver_verifies_it_independently() {
    let s = start("wh-sign", |_| {});
    let rx = Receiver::start();
    let (id, secret) = hook(&s, &rx.url(), &["webhook.test"]);
    let event_id = send_test(&s, &id);
    let got = rx.wait_for(1, Duration::from_secs(10));
    assert_eq!(got.len(), 1);
    let r = &got[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/hook");
    assert_eq!(r.header("content-type"), Some("application/json"));
    assert!(
        r.header("user-agent")
            .unwrap()
            .starts_with("CapIA-Webhook/")
    );
    assert!(r.header("x-capia-signature").unwrap().starts_with("v1="));
    assert!(r.timestamp().abs_diff(now_s()) < 10);
    assert_eq!(r.header("x-capia-attempt"), Some("1"));
    assert_eq!(r.event_id(), event_id);
    // assinatura conferida por uma implementação independente E pela API pública
    assert!(r.signature_ok_independent(&secret));
    assert_eq!(r.verify(&secret, now_s()), Ok(()));
    assert!(!r.signature_ok_independent("whsec_wrong"));
    // corpo versionado
    let b = r.json();
    assert_eq!(b["id"], event_id);
    assert_eq!(b["type"], "webhook.test");
    assert_eq!(b["version"], 1);
    let at = b["occurred_at"].as_str().unwrap();
    assert!(
        at.len() == 24 && at.ends_with('Z') && at.as_bytes()[10] == b'T',
        "{at}"
    );
    assert!(b["occurred_ms"].as_u64().unwrap() > 1_600_000_000_000);
    assert!(b["project_id"].is_null() && b["run_id"].is_null() && b["export_id"].is_null());
    assert_eq!(b["data"]["webhook_id"], id);
    // log de entrega
    assert!(wait_until(Duration::from_secs(5), || first_state(&s, &id)
        == "delivered"));
    let d = &deliveries(&s, &id)[0];
    for k in [
        "id",
        "webhook_id",
        "event_seq",
        "event_id",
        "attempt",
        "state",
        "next_attempt_ms",
        "last_status",
        "last_latency_ms",
        "last_error",
        "created_ms",
        "updated_ms",
    ] {
        assert!(d.get(k).is_some(), "delivery log lacks `{k}`: {d}");
    }
    assert_eq!(d["attempt"], 1);
    assert_eq!(d["last_status"], 200);
    assert!(d["last_latency_ms"].is_u64());
    assert!(d["last_error"].is_null());
    assert_eq!(r.header("x-capia-delivery").unwrap(), d["id"].to_string());
    assert_eq!(d["event_id"], event_id);
    // paginação do log
    let page = s
        .call(
            "GET",
            &format!("/v1/webhooks/{id}/deliveries?limit=1"),
            None,
        )
        .json();
    assert_eq!(page["deliveries"].as_array().unwrap().len(), 1);
}

#[test]
fn a_flaky_endpoint_is_retried_with_exponential_backoff_and_stable_ids() {
    let s = start("wh-retry", |c| {
        c.webhook_backoff_base = Duration::from_millis(100);
        c.webhook_backoff_cap = Duration::from_millis(2000);
    });
    let rx = Receiver::start();
    rx.script(|r| Action::status(if r.n < 2 { 500 } else { 200 }));
    let (id, secret) = hook(&s, &rx.url(), &["webhook.test"]);
    let event_id = send_test(&s, &id);
    let got = rx.wait_for(3, Duration::from_secs(15));
    assert_eq!(got.len(), 3, "{got:?}");
    assert!(wait_until(Duration::from_secs(5), || first_state(&s, &id)
        == "delivered"));
    // backoff: base*2^(n-1) com ±20% de jitter ⇒ ≥ 80 ms e ≥ 160 ms entre as tentativas
    let g1 = got[1].received_at - got[0].received_at;
    let g2 = got[2].received_at - got[1].received_at;
    assert!(g1 >= Duration::from_millis(70), "{g1:?}");
    assert!(g2 >= Duration::from_millis(150), "{g2:?}");
    assert!(g2 > g1, "{g1:?} {g2:?}");
    assert!(g2 < Duration::from_millis(2500));
    // ids estáveis entre tentativas; tentativa e assinatura variam
    let delivery = got[0].header("x-capia-delivery").unwrap();
    for (i, r) in got.iter().enumerate() {
        assert_eq!(r.event_id(), event_id);
        assert_eq!(r.header("x-capia-delivery").unwrap(), delivery);
        assert_eq!(r.header("x-capia-attempt").unwrap(), (i + 1).to_string());
        assert!(r.signature_ok_independent(&secret));
        assert_eq!(r.json()["id"], event_id);
    }
    let d = &deliveries(&s, &id)[0];
    assert_eq!(d["attempt"], 3);
    assert_eq!(d["state"], "delivered");
    assert_eq!(d["last_status"], 200);
}

#[test]
fn permanent_client_errors_die_immediately_but_throttling_is_retried() {
    let s = start("wh-perm", |_| {});
    let rx = Receiver::start();
    rx.script(|_| Action::status(410));
    let (id, _) = hook(&s, &rx.url(), &["webhook.test"]);
    send_test(&s, &id);
    assert!(wait_until(Duration::from_secs(10), || first_state(&s, &id) == "dead"));
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(rx.count(), 1, "a permanent 4xx must not be retried");
    let d = &deliveries(&s, &id)[0];
    assert_eq!(d["last_status"], 410);
    assert_eq!(d["attempt"], 1);
    assert!(d["last_error"].as_str().unwrap().contains("permanent"));

    let rx2 = Receiver::start();
    rx2.script(|r| Action::status(if r.n == 0 { 429 } else { 200 }));
    let (id2, _) = hook(&s, &rx2.url(), &["webhook.test"]);
    send_test(&s, &id2);
    assert!(wait_until(Duration::from_secs(10), || first_state(
        &s, &id2
    ) == "delivered"));
    assert_eq!(rx2.count(), 2);
}

#[test]
fn exhausted_retries_dead_letter_then_manual_redeliver() {
    let s = start("wh-dead", |c| c.webhook_max_attempts = 3);
    let rx = Receiver::start();
    let healthy = Arc::new(AtomicBool::new(false));
    let h = Arc::clone(&healthy);
    rx.script(move |_| Action::status(if h.load(Ordering::SeqCst) { 200 } else { 503 }));
    let (id, secret) = hook(&s, &rx.url(), &["webhook.test"]);
    send_test(&s, &id);
    assert!(wait_until(Duration::from_secs(15), || first_state(&s, &id) == "dead"));
    assert_eq!(rx.count(), 3);
    let d = deliveries(&s, &id)[0].clone();
    assert_eq!(d["attempt"], 3);
    assert_eq!(d["last_status"], 503);
    assert!(
        d["last_error"]
            .as_str()
            .unwrap()
            .starts_with("gave up after 3 attempts"),
        "{d}"
    );
    // nada mais chega sozinho
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(rx.count(), 3);
    // reentrega manual com o endpoint saudável
    healthy.store(true, Ordering::SeqCst);
    let did = d["id"].as_i64().unwrap();
    let r = s.call(
        "POST",
        &format!("/v1/webhooks/{id}/deliveries/{did}/redeliver"),
        None,
    );
    assert_eq!(r.status, 202, "{}", String::from_utf8_lossy(&r.body));
    assert!(wait_until(Duration::from_secs(10), || first_state(&s, &id)
        == "delivered"));
    let all = rx.requests();
    assert_eq!(all.len(), 4);
    assert_eq!(
        all[3].header("x-capia-attempt"),
        Some("1"),
        "redeliver restarts the count"
    );
    assert_eq!(all[3].event_id(), all[0].event_id());
    assert!(all[3].signature_ok_independent(&secret));
    // reentrega de entrega inexistente / de outro webhook
    assert_eq!(
        s.call(
            "POST",
            &format!("/v1/webhooks/{id}/deliveries/99999/redeliver"),
            None
        )
        .status,
        404
    );
    let (other, _) = hook(&s, &rx.url(), &["webhook.test"]);
    assert_eq!(
        s.call(
            "POST",
            &format!("/v1/webhooks/{other}/deliveries/{did}/redeliver"),
            None
        )
        .status,
        404
    );
}

#[test]
fn a_slow_endpoint_does_not_delay_other_webhooks_and_times_out() {
    let s = start("wh-slow", |c| {
        c.webhook.total_timeout = Duration::from_millis(1500);
        c.webhook_backoff_base = Duration::from_secs(30);
        c.webhook_backoff_cap = Duration::from_secs(30);
    });
    let slow = Receiver::start();
    slow.script(|_| Action::slow(200, Duration::from_millis(3500)));
    let fast = Receiver::start();
    let (slow_id, _) = hook(&s, &slow.url(), &["webhook.test"]);
    let (fast_id, _) = hook(&s, &fast.url(), &["webhook.test"]);
    for _ in 0..6 {
        send_test(&s, &slow_id);
    }
    assert!(!slow.wait_for(6, Duration::from_secs(5)).is_empty());
    let t0 = Instant::now();
    send_test(&s, &fast_id);
    let got = fast.wait_for(1, Duration::from_secs(5));
    assert_eq!(got.len(), 1);
    assert!(
        t0.elapsed() < Duration::from_millis(1200),
        "fast endpoint waited {:?} behind the slow one",
        t0.elapsed()
    );
    assert!(wait_until(Duration::from_secs(5), || first_state(
        &s, &fast_id
    ) == "delivered"));
    // o lento estoura o prazo: nova tentativa agendada, erro legível
    assert!(wait_until(Duration::from_secs(8), || {
        deliveries(&s, &slow_id)
            .iter()
            .all(|d| d["state"] == "retrying")
    }));
    let d = &deliveries(&s, &slow_id)[0];
    assert!(
        d["last_error"].as_str().unwrap().contains("timed out"),
        "{d}"
    );
    assert!(d["next_attempt_ms"].as_u64().unwrap() > 1_600_000_000_000);
}

#[test]
fn redirects_are_never_followed() {
    let s = start("wh-redirect", |_| {});
    let target = Receiver::start();
    let rx = Receiver::start();
    let to = target.url();
    rx.script(move |_| Action::redirect(&to));
    let (id, _) = hook(&s, &rx.url(), &["webhook.test"]);
    send_test(&s, &id);
    assert!(wait_until(Duration::from_secs(10), || first_state(&s, &id) == "dead"));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(rx.count(), 1);
    assert_eq!(
        target.count(),
        0,
        "the signed POST must never reach the redirect target"
    );
    let d = &deliveries(&s, &id)[0];
    assert_eq!(d["last_status"], 302);
    assert!(d["last_error"].as_str().unwrap().contains("redirect"));
}

#[test]
fn events_are_deduplicated_and_routed_by_subscription() {
    let s = start("wh-dup", |_| {});
    let all = Receiver::start();
    let only = Receiver::start();
    let (all_id, _) = hook(&s, &all.url(), &["*"]);
    let (only_id, _) = hook(&s, &only.url(), &["run.completed"]);
    let core = s.core();
    for _ in 0..3 {
        core.emit(
            "dup.1",
            "run.completed",
            Some("prj_x"),
            Some("run_x"),
            None,
            &json!({"n": 1}),
        );
    }
    core.emit(
        "dup.2",
        "run.failed",
        Some("prj_x"),
        Some("run_y"),
        None,
        &json!({"n": 2}),
    );
    all.wait_for(2, Duration::from_secs(10));
    only.wait_for(1, Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(all.count(), 2, "{:?}", all.requests().len());
    assert_eq!(only.count(), 1);
    assert_eq!(deliveries(&s, &all_id).len(), 2);
    assert_eq!(deliveries(&s, &only_id).len(), 1);
    let r = &only.requests()[0];
    assert_eq!(r.event_id(), "dup.1");
    let b = r.json();
    assert_eq!(b["project_id"], "prj_x");
    assert_eq!(b["run_id"], "run_x");
    assert_eq!(b["data"]["n"], 1);
}

#[test]
fn a_disabled_webhook_is_dead_lettered_with_a_clear_reason() {
    let s = start("wh-disabled", |_| {});
    let rx = Receiver::start();
    let (id, _) = hook(&s, &rx.url(), &["webhook.test"]);
    let r = s.call(
        "PATCH",
        &format!("/v1/webhooks/{id}"),
        Some(json!({"enabled": false})),
    );
    assert_eq!(r.status, 200);
    // eventos normais nem enfileiram para um webhook desabilitado
    s.core()
        .emit("off.1", "webhook.test", None, None, None, &json!({}));
    send_test(&s, &id);
    assert!(wait_until(Duration::from_secs(10), || first_state(&s, &id) == "dead"));
    let ds = deliveries(&s, &id);
    assert_eq!(ds.len(), 1);
    assert_eq!(ds[0]["last_error"], "webhook disabled");
    assert_eq!(rx.count(), 0);
    // reabilitar + reentregar funciona
    s.call(
        "PATCH",
        &format!("/v1/webhooks/{id}"),
        Some(json!({"enabled": true})),
    );
    let did = ds[0]["id"].as_i64().unwrap();
    s.call(
        "POST",
        &format!("/v1/webhooks/{id}/deliveries/{did}/redeliver"),
        None,
    );
    assert_eq!(rx.wait_for(1, Duration::from_secs(10)).len(), 1);
}

#[test]
fn secret_unavailable_after_restart_dead_letters_then_rotation_requeues() {
    let mut s = start("wh-secret", |_| {});
    let rx = Receiver::start();
    let (id, old_secret) = hook(&s, &rx.url(), &["webhook.test"]);
    s.shutdown();
    let dir = std::mem::replace(&mut s.dir, TempDir::new("unused"));
    // novo processo, cofre vazio (como um host sem cofre do SO)
    let s2 = start_in(dir, |_| {});
    send_test(&s2, &id);
    assert!(wait_until(Duration::from_secs(10), || first_state(
        &s2, &id
    ) == "dead"));
    let d = &deliveries(&s2, &id)[0];
    assert!(
        d["last_error"]
            .as_str()
            .unwrap()
            .starts_with("SECRET_UNAVAILABLE"),
        "{d}"
    );
    assert_eq!(rx.count(), 0);
    assert_eq!(
        s2.call("GET", &format!("/v1/webhooks/{id}"), None).json()["webhook"]["secret_configured"],
        false
    );
    // rotacionar devolve a entrega à fila e assina com o NOVO segredo
    let rot = s2.call("POST", &format!("/v1/webhooks/{id}/rotate-secret"), None);
    assert_eq!(rot.status, 200, "{}", String::from_utf8_lossy(&rot.body));
    let rot = rot.json();
    assert_eq!(rot["requeued"], 1);
    let new_secret = rot["secret"].as_str().unwrap().to_owned();
    assert_ne!(new_secret, old_secret);
    let got = rx.wait_for(1, Duration::from_secs(10));
    assert_eq!(got.len(), 1);
    assert!(got[0].signature_ok_independent(&new_secret));
    assert!(!got[0].signature_ok_independent(&old_secret));
    assert!(wait_until(Duration::from_secs(5), || first_state(&s2, &id)
        == "delivered"));
}

#[test]
fn the_url_is_revalidated_on_every_delivery() {
    let mut s = start("wh-revalidate", |_| {});
    let rx = Receiver::start();
    let (id, _) = hook(&s, &rx.url(), &["webhook.test"]);
    s.shutdown();
    let dir = std::mem::replace(&mut s.dir, TempDir::new("unused"));
    // a política mudou (loopback deixou de ser permitido): a URL gravada não vale mais
    let s2 = start_in(dir, |c| c.webhook.allow_loopback = false);
    send_test(&s2, &id);
    assert!(wait_until(Duration::from_secs(10), || first_state(
        &s2, &id
    ) == "dead"));
    let d = &deliveries(&s2, &id)[0];
    assert!(
        d["last_error"]
            .as_str()
            .unwrap()
            .starts_with("URL_REJECTED"),
        "{d}"
    );
    assert_eq!(
        rx.count(),
        0,
        "nothing may be sent to a URL that no longer passes the policy"
    );
}

#[test]
fn a_shutdown_mid_delivery_is_recovered_at_least_once() {
    let store: Arc<dyn capia_secrets::SecretStore> = Arc::new(capia_secrets::MemoryStore::new());
    let st = Arc::clone(&store);
    let mut s = start("wh-crash", move |c| c.secrets = st);
    let rx = Receiver::start();
    rx.script(|r| {
        if r.n == 0 {
            Action::slow(200, Duration::from_secs(8))
        } else {
            Action::status(200)
        }
    });
    let (id, secret) = hook(&s, &rx.url(), &["webhook.test"]);
    send_test(&s, &id);
    assert_eq!(rx.wait_for(1, Duration::from_secs(10)).len(), 1);
    s.shutdown();
    let dir = std::mem::replace(&mut s.dir, TempDir::new("unused"));
    let s2 = start_in(dir, move |c| c.secrets = store);
    assert!(wait_until(Duration::from_secs(15), || first_state(
        &s2, &id
    ) == "delivered"));
    let got = rx.requests();
    assert!(
        got.len() >= 2,
        "the in-flight delivery must be re-sent after the restart"
    );
    assert_eq!(got[0].event_id(), got[1].event_id());
    assert_eq!(
        got[0].header("x-capia-delivery"),
        got[1].header("x-capia-delivery")
    );
    // o cofre sobreviveu ao reinício: a MESMA chave assina a reentrega
    assert!(got.last().unwrap().signature_ok_independent(&secret));
}

#[test]
fn receiver_side_forgery_replay_and_tampering_are_rejected() {
    let s = start("wh-attack", |_| {});
    let rx = Receiver::start();
    let (id, secret) = hook(&s, &rx.url(), &["webhook.test"]);
    send_test(&s, &id);
    send_test(&s, &id);
    let got = rx.wait_for(2, Duration::from_secs(10));
    assert_eq!(got.len(), 2);
    let (a, b) = (&got[0], &got[1]);
    let now = now_s();
    assert_eq!(a.verify(&secret, now), Ok(()));
    // corpo adulterado
    let mut tampered = a.clone();
    tampered.body = a.json().to_string().replace("test", "tesT").into_bytes();
    assert_eq!(
        tampered.verify(&secret, now),
        Err(VerifyError::SignatureMismatch)
    );
    // um byte a mais (espaço no fim)
    let mut padded = a.clone();
    padded.body.push(b' ');
    assert_eq!(
        padded.verify(&secret, now),
        Err(VerifyError::SignatureMismatch)
    );
    // forjado sem o segredo / segredo errado
    assert_eq!(
        a.verify("whsec_guess", now),
        Err(VerifyError::SignatureMismatch)
    );
    // assinatura de OUTRO evento no corpo deste
    let mut swapped = a.clone();
    swapped
        .headers
        .retain(|(k, _)| !k.eq_ignore_ascii_case("x-capia-signature"));
    swapped.headers.push((
        "X-CapIA-Signature".into(),
        b.header("x-capia-signature").unwrap().to_owned(),
    ));
    assert_eq!(
        swapped.verify(&secret, now),
        Err(VerifyError::SignatureMismatch)
    );
    // timestamp trocado para "agora" com a assinatura antiga
    let mut retimed = a.clone();
    for (k, v) in &mut retimed.headers {
        if k.eq_ignore_ascii_case("x-capia-timestamp") {
            *v = (a.timestamp() + 1).to_string();
        }
    }
    assert_eq!(
        retimed.verify(&secret, now),
        Err(VerifyError::SignatureMismatch)
    );
    // replay fora da janela de 300 s (nos dois sentidos)
    assert_eq!(
        a.verify(&secret, now + 301),
        Err(VerifyError::TimestampOutsideTolerance)
    );
    assert_eq!(
        a.verify(&secret, now.saturating_sub(400)),
        Err(VerifyError::TimestampOutsideTolerance)
    );
    // cabeçalhos malformados / ausentes
    for h in [
        "",
        "v1=",
        "v1=zz",
        "v2=abcd",
        "sha256=00",
        &format!("v1={}", "g".repeat(64)),
    ] {
        let mut m = a.clone();
        m.headers
            .retain(|(k, _)| !k.eq_ignore_ascii_case("x-capia-signature"));
        m.headers.push(("X-CapIA-Signature".into(), h.to_owned()));
        assert_eq!(m.verify(&secret, now), Err(VerifyError::Malformed), "{h:?}");
    }
    // replay DENTRO da janela passa na assinatura: o receptor deduplica pelo event id
    let mut seen = std::collections::HashSet::new();
    assert!(seen.insert(a.event_id().to_owned()));
    assert_eq!(a.verify(&secret, now), Ok(()));
    assert!(
        !seen.insert(a.event_id().to_owned()),
        "second delivery of the same id is a duplicate"
    );
    assert_ne!(a.event_id(), b.event_id());
}
