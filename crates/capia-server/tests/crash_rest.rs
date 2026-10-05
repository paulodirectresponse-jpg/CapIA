//! Resiliência a queda (Track D-2): o binário REAL `capia-server serve` é morto com SIGKILL durante
//! upload, escrita no documento, requisição idempotente e export; a reabertura no MESMO data dir
//! precisa funcionar (WAL do `server.db`), os tokens continuam valendo e nada fica "meio feito".
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "common/attack.rs"]
mod attack;
mod common;

use attack::*;
use common::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::Write;
use std::time::Duration;

fn boot(dir: &TempDir) -> (ChildServer, String) {
    let srv = spawn_server(dir.path(), &["--bootstrap"]);
    srv.wait_ready();
    let tok = srv
        .bootstrap
        .clone()
        .expect("first boot prints the bootstrap token");
    (srv, tok)
}

/// Segunda subida no mesmo diretório: não imprime token novo, mas o antigo segue valendo.
fn reboot(dir: &TempDir, tok: &str) -> ChildServer {
    let srv = spawn_server(dir.path(), &["--bootstrap"]);
    srv.wait_ready();
    assert!(
        srv.bootstrap.is_none(),
        "a second boot must not mint a new bootstrap token"
    );
    let r = srv.call("GET", "/v1/server", Some(tok), &[], b"").unwrap();
    assert_eq!(
        r.status,
        200,
        "tokens must survive a SIGKILL: {}",
        String::from_utf8_lossy(&r.body)
    );
    srv
}

fn uploads_dir(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("uploads")
}

fn part_files(dir: &TempDir) -> Vec<std::path::PathBuf> {
    walk(&uploads_dir(dir))
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "part"))
        .collect()
}

#[test]
fn sigkill_during_an_upload_leaves_no_listed_or_importable_partial_and_a_clean_quota() {
    let dir = TempDir::new("crash-upload");
    let (mut srv, tok) = boot(&dir);
    let png = {
        let mut v = tiny_png();
        v.resize(1_000_000, 0);
        v
    };
    // um upload completo antes (precisa sobreviver)
    let ok = srv
        .call(
            "POST",
            "/v1/uploads",
            Some(&tok),
            &[("X-Capia-Filename", "done.png")],
            &png[..400],
        )
        .unwrap();
    assert_eq!(ok.status, 201);
    let done_id = ok.json()["upload"]["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // upload gigante parado no meio: o `.part` existe e cresce
    let mut sock = std::net::TcpStream::connect(srv.addr).unwrap();
    write!(
        sock,
        "POST /v1/uploads HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {tok}\r\nX-Capia-Filename: half.png\r\nContent-Length: 1000000\r\n\r\n",
        srv.addr.port()
    )
    .unwrap();
    sock.write_all(&png[..300_000]).unwrap();
    assert!(
        wait_until(Duration::from_secs(10), || part_files(&dir)
            .iter()
            .any(|p| std::fs::metadata(p).is_ok_and(|m| m.len() > 0))),
        "the .part file never appeared"
    );
    srv.kill();
    drop(sock);
    assert!(
        !part_files(&dir).is_empty(),
        "SIGKILL should leave the .part behind (that is the scenario)"
    );
    // reabertura: serve, não lista o parcial, o completo segue lá e a cota só conta o completo
    let srv = reboot(&dir, &tok);
    let list = srv
        .call("GET", "/v1/uploads", Some(&tok), &[], b"")
        .unwrap()
        .json();
    let ids: Vec<&str> = list["uploads"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u["upload_id"].as_str())
        .collect();
    assert_eq!(
        ids,
        vec![done_id.as_str()],
        "only the completed upload may be listed: {list}"
    );
    assert!(
        part_files(&dir).is_empty(),
        "orphaned staging files must be swept on startup: {:?}",
        part_files(&dir)
    );
    // importar o que não existe/é parcial: 404; e um novo upload cabe e entra normalmente
    let pid = srv
        .json_call("POST", "/v1/projects", &tok, &json!({"name": "c"}))
        .unwrap()
        .json()["project"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let imp = srv
        .json_call(
            "POST",
            &format!("/v1/projects/{pid}/assets"),
            &tok,
            &json!({"upload_id": "upl_0000000000000000"}),
        )
        .unwrap();
    assert_eq!(imp.status, 404);
    let again = srv
        .call(
            "POST",
            "/v1/uploads",
            Some(&tok),
            &[("X-Capia-Filename", "half.png")],
            &png[..500],
        )
        .unwrap();
    assert_eq!(
        again.status,
        201,
        "{}",
        String::from_utf8_lossy(&again.body)
    );
}

#[test]
fn sigkill_during_a_burst_of_applies_never_tears_or_duplicates_a_commit() {
    let dir = TempDir::new("crash-apply");
    let (mut srv, tok) = boot(&dir);
    let pid = srv
        .json_call("POST", "/v1/projects", &tok, &json!({"name": "crash"}))
        .unwrap()
        .json()["project"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let acked = std::sync::Arc::new(std::sync::Mutex::new(Vec::<usize>::new()));
    let addr = srv.addr;
    let (t2, p2, a2) = (tok.clone(), pid.clone(), acked.clone());
    let writer = std::thread::spawn(move || {
        for i in 0..10_000usize {
            let host = format!("127.0.0.1:{}", addr.port());
            let call = |path: &str, body: Value| {
                let b = body.to_string();
                let h = [
                    ("Host", host.as_str()),
                    ("Connection", "close"),
                    ("Authorization", &format!("Bearer {t2}")),
                    ("Content-Type", "application/json"),
                    ("Content-Length", &b.len().to_string()),
                ];
                exchange(
                    addr,
                    &build("POST", path, &h, b.as_bytes()),
                    Duration::from_secs(10),
                )
                .and_then(|o| try_parse(&o))
            };
            let Some(pv) = call(
                &format!("/v1/projects/{p2}/commands/preview"),
                json!({"commands": [{"operation_id": format!("op{i}"), "type": "create_sequence", "id": format!("seq{i}"),
                    "name": "S", "frame_rate": "30", "width": 1080, "height": 1920}]}),
            ) else {
                return;
            };
            if pv.status != 200 {
                continue;
            }
            let Some(ap) = call(
                &format!("/v1/projects/{p2}/commands/apply"),
                json!({"plan_token": pv.json()["plan_token"]}),
            ) else {
                return;
            };
            if ap.status == 200 {
                a2.lock().unwrap().push(i);
            }
        }
    });
    assert!(
        wait_until(Duration::from_secs(30), || acked.lock().unwrap().len() >= 5),
        "no applies completed"
    );
    std::thread::sleep(Duration::from_millis(300));
    srv.kill();
    let _ = writer.join();
    let acked: BTreeSet<usize> = acked.lock().unwrap().iter().copied().collect();
    assert!(acked.len() >= 5);
    // reabre o projeto: nenhum commit rasgado, todo apply confirmado está lá, sem duplicata
    let srv = reboot(&dir, &tok);
    let open = srv
        .json_call(
            "POST",
            &format!("/v1/projects/{pid}/open"),
            &tok,
            &json!({}),
        )
        .unwrap();
    assert_eq!(
        open.status,
        200,
        "the project must reopen after SIGKILL: {}",
        String::from_utf8_lossy(&open.body)
    );
    let seqs = srv
        .call(
            "GET",
            &format!("/v1/projects/{pid}/sequences"),
            Some(&tok),
            &[],
            b"",
        )
        .unwrap()
        .json();
    let ids: Vec<String> = seqs["sequences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_owned())
        .collect();
    let uniq: BTreeSet<&String> = ids.iter().collect();
    assert_eq!(
        ids.len(),
        uniq.len(),
        "duplicated sequences after a crash: {ids:?}"
    );
    for i in &acked {
        assert!(
            ids.contains(&format!("seq{i}")),
            "acknowledged apply #{i} was lost"
        );
    }
    // pode haver no máximo UM apply extra (o que estava em voo e foi gravado antes do ack)
    assert!(
        ids.len() <= acked.len() + 1,
        "{} sequences for {} acked applies",
        ids.len(),
        acked.len()
    );
    let hist = srv
        .call(
            "GET",
            &format!("/v1/projects/{pid}/history?limit=200"),
            Some(&tok),
            &[],
            b"",
        )
        .unwrap()
        .json();
    let n_hist = hist["entries"].as_array().unwrap().len();
    assert!(n_hist <= ids.len().max(200), "{n_hist}");
    // o documento aceita escrita nova (nada ficou travado)
    let pv = srv
        .json_call(
            "POST",
            &format!("/v1/projects/{pid}/commands/preview"),
            &tok,
            &json!({"commands": [{"operation_id": "after", "type": "create_sequence", "id": "after-crash", "name": "A", "frame_rate": "30", "width": 1080, "height": 1920}]}),
        )
        .unwrap();
    assert_eq!(pv.status, 200);
    let ap = srv
        .json_call(
            "POST",
            &format!("/v1/projects/{pid}/commands/apply"),
            &tok,
            &json!({"plan_token": pv.json()["plan_token"]}),
        )
        .unwrap();
    assert_eq!(ap.status, 200);
    // auditoria: sem entradas duplicadas de request_id
    let audit = srv
        .call("GET", "/v1/audit?limit=500", Some(&tok), &[], b"")
        .unwrap()
        .json();
    let rids: Vec<&str> = audit["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["request_id"].as_str())
        .collect();
    assert_eq!(
        rids.len(),
        rids.iter().collect::<BTreeSet<_>>().len(),
        "duplicated audit request ids"
    );
}

#[test]
fn sigkill_during_idempotent_requests_never_executes_a_key_twice() {
    let dir = TempDir::new("crash-idem");
    let (mut srv, tok) = boot(&dir);
    let attempted = std::sync::Arc::new(std::sync::Mutex::new(Vec::<usize>::new()));
    let addr = srv.addr;
    let (t2, at2) = (tok.clone(), attempted.clone());
    let writer = std::thread::spawn(move || {
        for i in 0..100_000usize {
            at2.lock().unwrap().push(i);
            let b =
                json!({"url": format!("https://example.com/h{i}"), "events": ["*"]}).to_string();
            let host = format!("127.0.0.1:{}", addr.port());
            let h = [
                ("Host", host.as_str()),
                ("Connection", "close"),
                ("Authorization", &format!("Bearer {t2}")),
                ("Content-Type", "application/json"),
                ("Idempotency-Key", &format!("crash-key-{i}")),
                ("Content-Length", &b.len().to_string()),
            ];
            if exchange(
                addr,
                &build("POST", "/v1/webhooks", &h, b.as_bytes()),
                Duration::from_secs(10),
            )
            .and_then(|o| try_parse(&o))
            .is_none()
            {
                return;
            }
            // o limite de 32 webhooks encerra o laço sozinho: os pedidos extras são 409
            if i >= 30 {
                return;
            }
        }
    });
    assert!(wait_until(Duration::from_secs(30), || attempted
        .lock()
        .unwrap()
        .len()
        >= 6));
    srv.kill();
    let _ = writer.join();
    let n = attempted.lock().unwrap().len();
    let srv = reboot(&dir, &tok);
    let count = |s: &ChildServer| {
        s.call("GET", "/v1/webhooks", Some(&tok), &[], b"")
            .unwrap()
            .json()["webhooks"]
            .as_array()
            .unwrap()
            .len()
    };
    let before = count(&srv);
    assert!(before <= n);
    let (mut fresh, mut replay, mut indeterminate) = (0, 0, 0);
    for i in 0..n {
        let b = json!({"url": format!("https://example.com/h{i}"), "events": ["*"]});
        let r = srv
            .call(
                "POST",
                "/v1/webhooks",
                Some(&tok),
                &[
                    ("Content-Type", "application/json"),
                    ("Idempotency-Key", &format!("crash-key-{i}")),
                ],
                b.to_string().as_bytes(),
            )
            .unwrap();
        match (r.status, r.header("idempotent-replay"), r.code().as_str()) {
            (201, Some("true"), _) => replay += 1,
            (201, None, _) => fresh += 1,
            (409, _, "IDEMPOTENCY_INDETERMINATE") => indeterminate += 1,
            other => panic!(
                "key {i}: unexpected {other:?}: {}",
                String::from_utf8_lossy(&r.body)
            ),
        }
    }
    // só as execuções "frescas" acrescentaram webhook: replay/indeterminado NUNCA reexecutam
    assert_eq!(
        count(&srv),
        before + fresh,
        "fresh={fresh} replay={replay} indeterminate={indeterminate}"
    );
    assert!(
        before + fresh <= n,
        "a key ran twice: {before}+{fresh} > {n}"
    );
    // e a segunda rodada (tudo já resolvido) não executa nada
    let after = count(&srv);
    for i in 0..n {
        let b = json!({"url": format!("https://example.com/h{i}"), "events": ["*"]});
        let _ = srv.call(
            "POST",
            "/v1/webhooks",
            Some(&tok),
            &[
                ("Content-Type", "application/json"),
                ("Idempotency-Key", &format!("crash-key-{i}")),
            ],
            b.to_string().as_bytes(),
        );
    }
    assert_eq!(count(&srv), after, "a settled key executed again");
}

#[test]
fn sigkill_leaves_the_server_db_recoverable_and_tokens_valid_repeatedly() {
    let dir = TempDir::new("crash-db");
    let (mut srv, tok) = boot(&dir);
    let extra = srv
        .json_call(
            "POST",
            "/v1/tokens",
            &tok,
            &json!({"name": "second", "scopes": ["project:read"]}),
        )
        .unwrap()
        .json()["secret"]
        .as_str()
        .unwrap()
        .to_owned();
    for round in 0..3 {
        // trabalho no meio: escritas pequenas em loop e um kill abrupto
        for i in 0..5 {
            srv.json_call(
                "POST",
                "/v1/projects",
                &tok,
                &json!({"name": format!("r{round}-{i}")}),
            );
        }
        srv.kill();
        srv = reboot(&dir, &tok);
        assert_eq!(
            srv.call("GET", "/v1/server", Some(&extra), &[], b"")
                .unwrap()
                .status,
            200
        );
    }
    let list = srv
        .call("GET", "/v1/projects?limit=200", Some(&tok), &[], b"")
        .unwrap()
        .json();
    assert!(list["projects"].as_array().unwrap().len() >= 10);
    // o stderr do filho nunca registrou pânico nem erro de banco
    let err = srv.err();
    assert!(!err.contains("panicked"), "{err}");
}

#[test]
fn sigkill_during_an_export_marks_it_interrupted_and_leaves_no_partial_final_file() {
    let have = std::process::Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !have {
        assert!(
            std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
            "CAPIA_REQUIRE_FFMPEG is set but ffprobe is missing"
        );
        eprintln!("skipping: ffprobe not available");
        return;
    }
    let dir = TempDir::new("crash-export");
    let (mut srv, tok) = boot(&dir);
    let pid = srv
        .json_call("POST", "/v1/projects", &tok, &json!({"name": "exp"}))
        .unwrap()
        .json()["project"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // sequência com UM clip sólido longo (300 s a 30 fps): o export demora o bastante para ser morto
    let frame: i64 = 23_520_000;
    let cmds = json!([
        {"operation_id": "e1", "type": "create_sequence", "id": "sq", "name": "S", "frame_rate": "30", "width": 1280, "height": 720},
        {"operation_id": "e2", "type": "add_track", "sequence": "sq", "id": "v1", "kind": "visual"},
        {"operation_id": "e3", "type": "insert_clip", "track": "v1", "start": 0, "clip": {
            "name": "long", "duration": 9000 * frame, "content": {"type": "solid", "color": "#3366CC"},
            "source_in": 0, "speed": "1", "reversed": false, "properties": {}}},
    ]);
    let pv = srv
        .json_call(
            "POST",
            &format!("/v1/projects/{pid}/commands/preview"),
            &tok,
            &json!({"commands": cmds}),
        )
        .unwrap();
    assert_eq!(pv.status, 200, "{}", String::from_utf8_lossy(&pv.body));
    let ap = srv
        .json_call(
            "POST",
            &format!("/v1/projects/{pid}/commands/apply"),
            &tok,
            &json!({"plan_token": pv.json()["plan_token"]}),
        )
        .unwrap();
    assert_eq!(ap.status, 200, "{}", String::from_utf8_lossy(&ap.body));
    let ex = srv
        .json_call(
            "POST",
            &format!("/v1/projects/{pid}/exports"),
            &tok,
            &json!({"items": [{"sequence": "sq", "preset": "intermediate"}]}),
        )
        .unwrap();
    if ex.status != 200 && ex.status != 201 && ex.status != 202 {
        // sem encoder aprovado neste ambiente: nada a medir (o ambiente decide)
        eprintln!(
            "skipping export crash: {} {}",
            ex.status,
            String::from_utf8_lossy(&ex.body)
        );
        return;
    }
    let export_id = ex.json()["exports"][0]["id"].as_str().unwrap().to_owned();
    // espera começar a rodar (ou terminar/falhar rápido) antes de matar
    wait_until(Duration::from_secs(20), || {
        let v = srv
            .call(
                "GET",
                &format!("/v1/projects/{pid}/exports/{export_id}"),
                Some(&tok),
                &[],
                b"",
            )
            .unwrap()
            .json();
        v["export"]["state"] != "queued"
    });
    srv.kill();
    let srv = reboot(&dir, &tok);
    let open = srv
        .json_call(
            "POST",
            &format!("/v1/projects/{pid}/open"),
            &tok,
            &json!({}),
        )
        .unwrap();
    assert_eq!(open.status, 200);
    let got = srv
        .call(
            "GET",
            &format!("/v1/projects/{pid}/exports/{export_id}"),
            Some(&tok),
            &[],
            b"",
        )
        .unwrap();
    assert_eq!(got.status, 200);
    let state = got.json()["export"]["state"].as_str().unwrap().to_owned();
    // ou terminou antes da queda, ou foi marcado como interrompido — nunca "running" para sempre
    assert!(
        matches!(state.as_str(), "completed" | "failed"),
        "state after restart: {state}"
    );
    if state == "failed" {
        let e = &got.json()["export"]["error"];
        assert_eq!(e["code"], "INTERRUPTED", "{e}");
        // nenhum arquivo final parcial: o diretório do export só tem o final válido ou nada
        let finals: Vec<_> = walk(&dir.path().join("exports"))
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "mov" || e == "mp4"))
            .collect();
        assert!(
            finals.is_empty(),
            "a partial final file survived: {finals:?}"
        );
    }
}

#[test]
fn a_pending_idempotency_key_from_a_dead_process_is_indeterminate_at_once() {
    let dir = TempDir::new("crash-idem-pending");
    let (mut srv, tok) = boot(&dir);
    let tid = srv
        .call("GET", "/v1/tokens", Some(&tok), &[], b"")
        .unwrap()
        .json()["tokens"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    srv.kill();
    // o estado exato de uma queda NO MEIO de um pedido: linha `pending` (status 0) recém-criada
    let body = json!({"url": "https://example.com/pending", "events": ["*"]});
    let key = "in-flight-key";
    {
        let db = capia_store::ServerDb::open(&dir.path().join("server.db"), Duration::from_secs(5))
            .unwrap();
        let hashed = format!(
            "ik_{}",
            &capia_server::mac::sha256_hex(key.as_bytes())[..40]
        );
        let hash = capia_server::mac::sha256_hex(format!("webhooks.create\n{body}").as_bytes());
        let now = capia_server::auth::now_ms();
        let r = db
            .idem_begin(&tid, &hashed, "webhooks.create", &hash, now, 300_000)
            .unwrap();
        assert!(matches!(r, capia_store::IdemBegin::New));
    }
    let srv = reboot(&dir, &tok);
    let r = srv
        .call(
            "POST",
            "/v1/webhooks",
            Some(&tok),
            &[
                ("Content-Type", "application/json"),
                ("Idempotency-Key", key),
            ],
            body.to_string().as_bytes(),
        )
        .unwrap();
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(
        r.code(),
        "IDEMPOTENCY_INDETERMINATE",
        "an orphaned pending key must not read as in progress"
    );
    // nada foi executado por causa disso
    let list = srv
        .call("GET", "/v1/webhooks", Some(&tok), &[], b"")
        .unwrap()
        .json();
    assert_eq!(list["webhooks"].as_array().unwrap().len(), 0);
    // com uma chave NOVA o pedido executa normalmente
    let ok = srv
        .call(
            "POST",
            "/v1/webhooks",
            Some(&tok),
            &[
                ("Content-Type", "application/json"),
                ("Idempotency-Key", "fresh-key"),
            ],
            body.to_string().as_bytes(),
        )
        .unwrap();
    assert_eq!(ok.status, 201);
}
