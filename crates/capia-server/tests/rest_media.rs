//! Uploads em streaming → import pelo sistema de assets → leitura sem caminhos; SSE; shutdown e
//! reabertura. Precisa de FFmpeg/ffprobe (como os demais testes de mídia: `CAPIA_REQUIRE_FFMPEG=1`
//! transforma a ausência em falha).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/media")
        .join(name)
}

fn have_ffmpeg() -> bool {
    let ok = std::process::Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        assert!(
            std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
            "CAPIA_REQUIRE_FFMPEG is set but ffprobe is missing"
        );
        eprintln!("skipping: ffprobe not available");
    }
    ok
}

fn upload(s: &TestServer, token: &str, filename: &str, bytes: &[u8]) -> Resp {
    request(
        s.addr,
        "POST",
        "/v1/uploads",
        Some(token),
        &[
            ("Content-Type", "application/octet-stream"),
            ("X-Capia-Filename", filename),
        ],
        Some(bytes),
    )
}

fn wait_import(s: &TestServer, pid: &str, ticket: &str) -> Value {
    let t0 = Instant::now();
    loop {
        let r = s.call("GET", &format!("/v1/projects/{pid}/imports/{ticket}"), None);
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let v = r.json();
        let state = v["import"]["state"].as_str().unwrap_or_default().to_owned();
        if state != "pending" {
            return v;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(60),
            "import never finished: {v}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn upload_import_and_read_assets_without_leaking_paths() {
    if !have_ffmpeg() {
        return;
    }
    let s = start("media", |_| {});
    let pid = s.create_project("media");
    let bytes = std::fs::read(fixture("video_audio.mp4")).unwrap();
    let up = upload(&s, &s.admin, "../../clip final.mp4", &bytes);
    assert_eq!(up.status, 201, "{}", String::from_utf8_lossy(&up.body));
    let meta = up.json();
    let meta = &meta["upload"];
    assert_eq!(
        meta["kind"],
        "video",
        "{}",
        String::from_utf8_lossy(&up.body)
    );
    assert_eq!(
        meta["filename"], "clip final.mp4",
        "the name is sanitized to a basename"
    );
    assert_eq!(meta["size"].as_u64().unwrap(), bytes.len() as u64);
    assert_eq!(
        meta["sha256"].as_str().unwrap(),
        capia_server::uploads::sha_of(&bytes)
    );
    let id = meta["upload_id"].as_str().unwrap().to_owned();
    // idempotente por conteúdo: o mesmo arquivo do mesmo token não vira outro upload
    let again = upload(&s, &s.admin, "../../clip final.mp4", &bytes).json();
    assert_eq!(again["upload"]["upload_id"], id.as_str());
    assert_eq!(again["upload"]["deduplicated"], true);
    // checksum divergente é recusado (e nada fica em staging)
    let bad = request(
        s.addr,
        "POST",
        "/v1/uploads",
        Some(&s.admin),
        &[
            ("X-Capia-Filename", "x.mp4"),
            ("X-Capia-Sha256", &"0".repeat(64)),
        ],
        Some(&bytes),
    );
    assert_eq!(
        (bad.status, bad.code().as_str()),
        (422, "CHECKSUM_MISMATCH")
    );
    // conteúdo que não é mídia: 415 mesmo com extensão .mp4
    let exe = upload(
        &s,
        &s.admin,
        "movie.mp4",
        b"MZ\x90\0\x03\0\0\0\x04\0\0\0\xff\xff\0\0",
    );
    assert_eq!(exe.status, 415);
    assert_eq!(
        s.call("GET", "/v1/uploads", None).json()["uploads"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let imp = s.call(
        "POST",
        &format!("/v1/projects/{pid}/assets"),
        Some(json!({"upload_id": id})),
    );
    assert_eq!(imp.status, 202, "{}", String::from_utf8_lossy(&imp.body));
    let ticket = imp.json()["ticket_id"].as_str().unwrap().to_owned();
    let done = wait_import(&s, &pid, &ticket);
    assert_eq!(done["import"]["state"], "finalized", "{done}");
    assert!(
        !done.to_string().contains("clip final.mp4")
            || !done["import"].as_object().unwrap().contains_key("path")
    );
    // o upload foi consumido (a mídia saiu do staging para a pasta durável do projeto)
    let twice = s.call(
        "POST",
        &format!("/v1/projects/{pid}/assets"),
        Some(json!({"upload_id": id})),
    );
    assert_eq!(twice.status, 409);
    assert_eq!(twice.code(), "UPLOAD_CONSUMED");
    // leitura: nenhum caminho do servidor
    let list = s.call("GET", &format!("/v1/projects/{pid}/assets"), None);
    let text = String::from_utf8_lossy(&list.body).into_owned();
    assert!(
        !text.contains(&s.dir.path().display().to_string()),
        "{text}"
    );
    let assets = list.json()["assets"].as_array().unwrap().clone();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0]["has_video"], true);
    assert!(assets[0].get("path").is_none());
    let one = s.call(
        "GET",
        &format!(
            "/v1/projects/{pid}/assets/{}",
            assets[0]["id"].as_str().unwrap()
        ),
        None,
    );
    assert_eq!(one.status, 200);
    assert_eq!(
        s.call("GET", &format!("/v1/projects/{pid}/assets/nope"), None)
            .status,
        404
    );
    // o evento `asset.imported` foi publicado (a bomba de eventos rodou o `pump` do engine)
    let t0 = Instant::now();
    loop {
        let ev = s.call("GET", "/v1/events", None).json();
        if ev["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["type"] == "asset.imported")
        {
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "no asset.imported event: {ev}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // documento inline pequeno + paginação
    let doc = s.call(
        "POST",
        "/v1/uploads/inline",
        Some(json!({"filename": "brief.txt", "content_base64": "Q29waWE6IHRlc3RlCg=="})),
    );
    assert_eq!(doc.status, 201, "{}", String::from_utf8_lossy(&doc.body));
    assert_eq!(doc.json()["upload"]["kind"], "document");
    let p1 = s.call("GET", "/v1/uploads?limit=1", None).json();
    assert_eq!(p1["uploads"].as_array().unwrap().len(), 1);
    assert!(p1["next"].is_string() || p1["uploads"].as_array().unwrap().len() == 1);
    // documento não vira asset de mídia
    let doc_id = doc.json()["upload"]["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = s.call(
        "POST",
        &format!("/v1/projects/{pid}/assets"),
        Some(json!({"upload_id": doc_id})),
    );
    assert_eq!(r.status, 422);
    assert_eq!(
        s.call("DELETE", &format!("/v1/uploads/{doc_id}"), None)
            .status,
        200
    );
    assert_eq!(
        s.call("DELETE", &format!("/v1/uploads/{doc_id}"), None)
            .status,
        404
    );
}

#[test]
fn upload_limits_quota_and_stalled_clients_are_enforced() {
    let s = start("limits", |c| {
        c.max_upload_bytes = 64 * 1024;
        c.upload_quota_bytes = 100 * 1024;
    });
    let png = |n: usize| {
        let mut v = vec![0x89, b'P', b'N', b'G', 0, 0, 0, 0, 0, 0, 0, 0];
        v.resize(n, 7);
        v
    };
    assert_eq!(upload(&s, &s.admin, "a.png", &png(2000)).status, 201);
    // declarado grande demais: recusado antes de ler
    let big = upload(&s, &s.admin, "big.png", &png(70 * 1024));
    assert_eq!((big.status, big.code().as_str()), (413, "UPLOAD_TOO_LARGE"));
    // cota: 60 KiB cabem; o próximo não
    assert_eq!(upload(&s, &s.admin, "b.png", &png(60 * 1024)).status, 201);
    let q = upload(&s, &s.admin, "c.png", &png(60 * 1024 - 1));
    assert_eq!(
        (q.status, q.code().as_str()),
        (507, "STORAGE_QUOTA_EXCEEDED")
    );
    // sem Content-Length: 411
    let raw411 = raw(
        s.addr,
        format!(
            "POST /v1/uploads HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nX-Capia-Filename: a.png\r\nConnection: close\r\n\r\n",
            s.addr.port(),
            s.admin
        )
        .as_bytes(),
    );
    assert_eq!(parse(&raw411).status, 411);
    // sem scope: recusado SEM ler o corpo
    let ro = s.token("ro", &["project:read"]);
    assert_eq!(upload(&s, &ro, "d.png", &png(100)).status, 403);
    // sem nome
    let r = request(
        s.addr,
        "POST",
        "/v1/uploads",
        Some(&s.admin),
        &[],
        Some(&png(100)),
    );
    assert_eq!(r.status, 400);
    // cliente que trava no meio do corpo: o servidor desiste (408) e libera o slot/worker
    let mut c = std::net::TcpStream::connect(s.addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let head = format!(
        "POST /v1/uploads HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nX-Capia-Filename: slow.png\r\nContent-Length: 5000\r\nConnection: close\r\n\r\n",
        s.addr.port(),
        s.admin
    );
    c.write_all(head.as_bytes()).unwrap();
    c.write_all(&png(500)).unwrap();
    let mut out = Vec::new();
    let _ = c.read_to_end(&mut out);
    let resp = parse(&out);
    assert_eq!(resp.status, 408, "{}", String::from_utf8_lossy(&resp.body));
    // nada ficou pela metade em staging
    let listed = s.call("GET", "/v1/uploads", None).json();
    assert!(
        listed["uploads"]
            .as_array()
            .unwrap()
            .iter()
            .all(|u| u["filename"] != "slow.png")
    );
    let leftovers: Vec<_> = std::fs::read_dir(s.dir.path().join("uploads"))
        .unwrap()
        .flatten()
        .filter(|e| {
            std::fs::read_dir(e.path())
                .unwrap()
                .flatten()
                .any(|f| f.file_name().to_string_lossy().ends_with(".part"))
        })
        .collect();
    assert!(leftovers.is_empty(), "partial uploads must be removed");
}

#[test]
fn the_event_stream_delivers_events_and_respects_scope_and_limits() {
    let s = start("sse", |c| c.max_sse = 1);
    let hook = s.call(
        "POST",
        "/v1/webhooks",
        Some(json!({"url": "http://127.0.0.1:9/hook", "events": ["*"]})),
    );
    assert_eq!(hook.status, 201, "{}", String::from_utf8_lossy(&hook.body));
    let wid = hook.json()["webhook"]["id"].as_str().unwrap().to_owned();
    let mut c = std::net::TcpStream::connect(s.addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(
        c,
        "GET /v1/events/stream HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\n\r\n",
        s.addr.port(),
        s.admin
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    // o segundo stream estoura o limite (max_sse = 1)
    let second = request(
        s.addr,
        "GET",
        "/v1/events/stream",
        Some(&s.admin),
        &[],
        None,
    );
    assert_eq!(second.status, 429);
    // sem scope: 403
    let ro = s.token("nope", &["media:read"]);
    assert_eq!(
        request(s.addr, "GET", "/v1/events/stream", Some(&ro), &[], None).status,
        403
    );
    assert_eq!(
        request(s.addr, "GET", "/v1/events/stream", None, &[], None).status,
        401
    );
    assert_eq!(
        s.call("POST", &format!("/v1/webhooks/{wid}/test"), None)
            .status,
        202
    );
    let t0 = Instant::now();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    while !String::from_utf8_lossy(&buf).contains("event: webhook.test") {
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "{}",
            String::from_utf8_lossy(&buf)
        );
        let n = c.read(&mut chunk).unwrap();
        assert!(n > 0, "stream closed early");
        buf.extend_from_slice(&chunk[..n]);
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    assert!(
        text.contains("content-type: text/event-stream")
            || text.contains("Content-Type: text/event-stream")
    );
    assert!(text.contains("\nid: "));
}

#[test]
fn a_graceful_shutdown_and_reopen_keep_tokens_projects_and_history() {
    let mut s = start("restart", |_| {});
    let pid = s.create_project("persist");
    let pv = s
        .call(
            "POST",
            &format!("/v1/projects/{pid}/commands/preview"),
            Some(json!({"commands": create_sequence_cmds("o1", "sx")})),
        )
        .json();
    assert_eq!(
        s.call(
            "POST",
            &format!("/v1/projects/{pid}/commands/apply"),
            Some(json!({"plan_token": pv["plan_token"]}))
        )
        .status,
        200
    );
    let admin = s.admin.clone();
    let dir = std::mem::replace(
        &mut s.dir,
        TempDir(std::env::temp_dir().join("capia-srv-unused")),
    );
    s.shutdown();
    assert!(
        !dir.path().join("server.json").exists(),
        "server.json is removed on a clean shutdown"
    );
    // reabre o MESMO diretório de dados: o token continua válido e o projeto volta
    let s2 = start_in_existing(dir);
    assert_eq!(
        s2.call_as(&admin, "GET", "/v1/projects", None).json()["projects"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        s2.call_as(&admin, "POST", &format!("/v1/projects/{pid}/open"), None)
            .status,
        200
    );
    let seqs = s2
        .call_as(
            &admin,
            "GET",
            &format!("/v1/projects/{pid}/sequences"),
            None,
        )
        .json();
    assert_eq!(seqs["sequences"].as_array().unwrap().len(), 1);
    assert_eq!(seqs["sequences"][0]["id"], "sx");
}

fn start_in_existing(dir: TempDir) -> TestServer {
    // `start_in` cria um token novo; o antigo continua válido porque o banco é o mesmo
    start_in(dir, |_| {})
}
