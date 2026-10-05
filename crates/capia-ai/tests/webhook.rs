//! Entrega de webhooks: destino endurecido (SSRF), sem redirect, timeouts, classificação de status.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_ai::webhook::{WebhookClient, WebhookPolicy};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

/// Servidor de uma resposta: devolve (porta, requisição crua recebida).
fn once(response: &'static str) -> (u16, mpsc::Receiver<String>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        if let Ok((mut s, _)) = l.accept() {
            let mut buf = vec![0u8; 16384];
            let n = s.read(&mut buf).unwrap_or(0);
            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
            let _ = s.write_all(response.as_bytes());
        }
    });
    (port, rx)
}

fn client() -> WebhookClient {
    WebhookClient::new(WebhookPolicy::default()).unwrap()
}

fn hdr(k: &str, v: &str) -> (String, String) {
    (k.to_owned(), v.to_owned())
}

#[tokio::test]
async fn a_2xx_is_delivered_with_headers_and_body_intact() {
    let (port, rx) = once("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");
    let d = client()
        .post(
            &format!("http://127.0.0.1:{port}/hook"),
            &[hdr("X-CapIA-Signature", "v1=abc")],
            br#"{"a":1}"#.to_vec(),
        )
        .await;
    assert!(d.delivered(), "{d:?}");
    let raw = rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .to_lowercase();
    assert!(raw.starts_with("post /hook"), "{raw}");
    assert!(raw.contains("x-capia-signature: v1=abc"));
    assert!(raw.contains("content-type: application/json"));
    assert!(raw.ends_with(r#"{"a":1}"#));
}

#[tokio::test]
async fn statuses_are_classified_retryable_or_permanent() {
    let (p, _r) = once("HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n");
    let d = client()
        .post(&format!("http://127.0.0.1:{p}/"), &[], vec![])
        .await;
    assert!(!d.delivered() && !d.permanent(), "{d:?}");
    let (p, _r) = once("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
    let d = client()
        .post(&format!("http://127.0.0.1:{p}/"), &[], vec![])
        .await;
    assert!(!d.delivered() && d.permanent(), "{d:?}");
    let (p, _r) = once("HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n");
    let d = client()
        .post(&format!("http://127.0.0.1:{p}/"), &[], vec![])
        .await;
    assert!(!d.delivered() && !d.permanent(), "429 is retryable: {d:?}");
}

#[tokio::test]
async fn redirects_are_never_followed() {
    let (target, rx_target) = once("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    let resp: &'static str = Box::leak(
        format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:{target}/stolen\r\nContent-Length: 0\r\n\r\n")
            .into_boxed_str(),
    );
    let (p, _r) = once(resp);
    let d = client()
        .post(
            &format!("http://127.0.0.1:{p}/"),
            &[hdr("X-CapIA-Signature", "v1=secret-derived")],
            b"{}".to_vec(),
        )
        .await;
    assert!(!d.delivered(), "{d:?}");
    assert_eq!(d.status, Some(307));
    assert!(d.error.unwrap().contains("redirect"));
    assert!(
        rx_target.recv_timeout(Duration::from_millis(400)).is_err(),
        "the redirect target must never receive the signed body"
    );
}

#[tokio::test]
async fn dangerous_destinations_are_refused_before_any_network() {
    let c = client();
    for url in [
        "http://169.254.169.254/latest/meta-data",
        "https://169.254.169.254/",
        "https://10.0.0.5/hook",
        "http://example.com/hook",
        "https://user:pass@example.com/hook",
        "file:///etc/passwd",
        "ftp://example.com/",
        "https://printer.local/hook",
    ] {
        let d = c.post(url, &[], vec![]).await;
        assert!(!d.delivered() && d.status.is_none(), "{url}: {d:?}");
        assert!(d.error.is_some(), "{url}");
    }
    // sem loopback permitido, nem o receptor local passa
    let strict = WebhookClient::new(WebhookPolicy {
        allow_loopback: false,
        ..WebhookPolicy::default()
    })
    .unwrap();
    let d = strict.post("http://127.0.0.1:9/", &[], vec![]).await;
    assert!(d.status.is_none() && d.error.is_some());
}

#[tokio::test]
async fn a_slow_endpoint_times_out_instead_of_holding_the_queue() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        if let Ok((_s, _)) = l.accept() {
            std::thread::sleep(Duration::from_secs(5));
        }
    });
    let c = WebhookClient::new(WebhookPolicy {
        total_timeout: Duration::from_millis(300),
        ..WebhookPolicy::default()
    })
    .unwrap();
    let t = std::time::Instant::now();
    let d = c
        .post(&format!("http://127.0.0.1:{port}/"), &[], vec![])
        .await;
    assert!(t.elapsed() < Duration::from_secs(3));
    assert!(!d.delivered() && d.error.unwrap().contains("timed out"));
}
