//! `SafeFetcher` (Asset Gateway): download contido — host permitido, redirect para fora recusado,
//! tipo/tamanho/hash, staging atômico, cancelamento, nenhuma credencial vazando.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_ai::CancelToken;
use capia_ai::fetch::{FetchPolicy, SafeFetcher};
use capia_ai::testkit::{MockResponse, MockServer};
use std::path::PathBuf;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("capia-fetch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn policy(host: &str) -> FetchPolicy {
    FetchPolicy {
        allowed_hosts: vec![host.into()],
        allow_loopback: true,
        max_bytes: 1_000_000,
        ..FetchPolicy::default()
    }
}

#[tokio::test]
async fn downloads_hashes_and_publishes_atomically() {
    let body: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
    let b2 = body.clone();
    let srv = MockServer::start(move |_| MockResponse::Bytes(200, "video/mp4".into(), b2.clone())).await;
    let dir = tmp("ok");
    let f = SafeFetcher::new(policy("127.0.0.1")).unwrap();
    let dest = dir.join("clip.mp4");
    let r = f
        .download(&format!("{}/a.mp4", srv.url()), &dest, &CancelToken::new())
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), body);
    assert_eq!(r.bytes, 5000);
    assert!(r.sha256.starts_with("sha256:") && r.sha256.len() == 71);
    assert!(!dest.with_extension("part").exists(), "no partial file is left behind");
    // o mesmo conteúdo ⇒ o mesmo hash (identidade = conteúdo)
    let dest2 = dir.join("clip2.mp4");
    let r2 = f
        .download(&format!("{}/b.mp4", srv.url()), &dest2, &CancelToken::new())
        .await
        .unwrap();
    assert_eq!(r.sha256, r2.sha256);
}

#[tokio::test]
async fn refuses_a_wrong_type_an_oversize_body_and_an_empty_file_without_leaving_files() {
    let dir = tmp("refuse");
    let f = SafeFetcher::new(policy("127.0.0.1")).unwrap();
    let html = MockServer::start(|_| MockResponse::Bytes(200, "text/html".into(), b"<html>".to_vec())).await;
    let d1 = dir.join("a.mp4");
    let e = f.download(&format!("{}/x", html.url()), &d1, &CancelToken::new()).await.unwrap_err();
    assert!(e.message.contains("content type"), "{e:?}");
    assert!(!d1.exists());
    let huge = MockServer::start(|_| MockResponse::HugeTyped("video/mp4".into(), 5_000_000)).await;
    let d2 = dir.join("b.mp4");
    let e = f.download(&format!("{}/x", huge.url()), &d2, &CancelToken::new()).await.unwrap_err();
    assert!(e.message.contains("exceeds"), "{e:?}");
    assert!(!d2.exists() && !d2.with_extension("part").exists());
    let empty = MockServer::start(|_| MockResponse::Bytes(200, "video/mp4".into(), vec![])).await;
    let d3 = dir.join("c.mp4");
    assert!(f.download(&format!("{}/x", empty.url()), &d3, &CancelToken::new()).await.is_err());
    assert!(!d3.exists());
    let not_found = MockServer::start(|_| MockResponse::Json(404, "{}".into())).await;
    assert!(f.download(&format!("{}/x", not_found.url()), &dir.join("d.mp4"), &CancelToken::new()).await.is_err());
}

#[tokio::test]
async fn a_redirect_to_a_host_outside_the_allow_list_is_not_followed() {
    // allow-list só 127.0.0.1; o servidor redireciona para `localhost` (outro host)
    let target = MockServer::start(|_| MockResponse::Bytes(200, "video/mp4".into(), vec![1, 2, 3])).await;
    let tport = target.url_localhost();
    let srv = MockServer::start(move |_| MockResponse::Redirect(format!("{tport}/secret.mp4"))).await;
    let dir = tmp("redir");
    let f = SafeFetcher::new(policy("127.0.0.1")).unwrap();
    let dest = dir.join("a.mp4");
    let e = f.download(&format!("{}/x", srv.url()), &dest, &CancelToken::new()).await;
    assert!(e.is_err());
    assert!(!dest.exists());
    assert!(target.seen().is_empty(), "the foreign host must never be contacted");
}

#[tokio::test]
async fn private_and_non_allowed_urls_never_reach_the_network() {
    let f = SafeFetcher::new(FetchPolicy {
        allowed_hosts: vec!["cdn.example.com".into()],
        ..FetchPolicy::default()
    })
    .unwrap();
    let dir = tmp("urls");
    for u in ["https://127.0.0.1/a.mp4", "file:///etc/passwd", "https://evil.test/a.mp4", "http://cdn.example.com/a.mp4"] {
        assert!(f.download(u, &dir.join("x.mp4"), &CancelToken::new()).await.is_err(), "{u}");
    }
}

#[tokio::test]
async fn cancelling_aborts_the_download_and_leaves_nothing() {
    let srv = MockServer::start(|_| MockResponse::Hang).await;
    let dir = tmp("cancel");
    let f = SafeFetcher::new(policy("127.0.0.1")).unwrap();
    let cancel = CancelToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        c2.cancel();
    });
    let dest = dir.join("a.mp4");
    let e = f.download(&format!("{}/x", srv.url()), &dest, &cancel).await.unwrap_err();
    assert!(e.code == capia_ai::ErrorCode::Cancelled, "{e:?}");
    assert!(!dest.exists());
}
