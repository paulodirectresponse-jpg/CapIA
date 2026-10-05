//! Fuzzing determinístico do REST `/v1` (Track D-2), sem dependência nova: PRNG com semente fixa
//! gera métodos/caminhos/cabeçalhos/corpos/queries malformados contra um servidor vivo.
//!
//! Invariantes por requisição: sempre há resposta ou fechamento limpo dentro de um prazo (nunca
//! hang); nenhum 5xx além dos documentados (501/503/505/507); o corpo nunca vaza caminho/SQL/pilha.
//! No fim: `/v1/health` + uma chamada autenticada válida respondem, threads/fds ficam limitados.
//!
//! `CAPIA_FUZZ_CASES` (padrão 400) e `CAPIA_FUZZ_SEED` ajustam a rodada. Todo caso que quebrar um
//! invariante vira um arquivo em `tests/fuzz-corpus/*.http` (reexecutado em todo `cargo test`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "common/attack.rs"]
mod attack;
mod common;

use attack::*;
use capia_server::catalog;
use common::*;
use std::time::{Duration, Instant};

const DOCUMENTED_5XX: [u16; 5] = [501, 503, 505, 507, 504];

fn env_num(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn fill(tpl: &str, rng: &mut Prng, pid: &str) -> String {
    tpl.split('/')
        .map(|seg| {
            if seg == "{project_id}" {
                if rng.chance(3) {
                    weird_segment(rng)
                } else {
                    pid.to_owned()
                }
            } else if seg.starts_with('{') {
                weird_segment(rng)
            } else {
                seg.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn weird_segment(rng: &mut Prng) -> String {
    const POOL: &[&str] = &[
        "x1",
        "..",
        ".",
        "%2e%2e",
        "%00",
        "%",
        "%zz",
        "a%2fb",
        "prj_0000",
        "upl_0",
        "tok_x",
        "whk_1",
        "CON",
        "\u{1f4a5}",
        "%e0%a4%a",
        "a b",
        "a?b",
        "a#b",
        "a;b",
        "~",
        "*",
        "0",
        "-1",
        "99999999999999999999",
        "../../etc/passwd",
        "%252e%252e",
        "{project_id}",
        "${x}",
        "<script>",
        "'--",
        "\"",
        "\\",
    ];
    if rng.chance(8) {
        return "a".repeat(1 + rng.below(6000));
    }
    (*rng.pick(POOL)).to_owned()
}

fn json_value(rng: &mut Prng, depth: usize) -> String {
    match rng.below(if depth > 4 { 7 } else { 10 }) {
        0 => "null".into(),
        1 => "true".into(),
        2 => format!("{}", rng.next_u64() as i64),
        3 => "99999999999999999999999999999".into(),
        4 => "1e999".into(),
        5 => "-0.0".into(),
        6 => {
            let n = rng.below(30);
            let s: String = (0..n)
                .map(|_| {
                    *rng.pick(&[
                        'a',
                        'Z',
                        '0',
                        '/',
                        '\\',
                        '"',
                        '\u{0}',
                        '\u{ff0e}',
                        '\u{1f4a5}',
                        ' ',
                        '%',
                        '$',
                        '{',
                    ])
                })
                .collect();
            serde_json::Value::String(s).to_string()
        }
        7 => format!(
            "[{}]",
            (0..rng.below(5))
                .map(|_| json_value(rng, depth + 1))
                .collect::<Vec<_>>()
                .join(",")
        ),
        8 => json_object(rng, depth + 1),
        _ => format!("\"{}\"", "x".repeat(rng.below(3000))),
    }
}

fn json_object(rng: &mut Prng, depth: usize) -> String {
    const KEYS: &[&str] = &[
        "name",
        "url",
        "events",
        "scopes",
        "plan_token",
        "commands",
        "upload_id",
        "filename",
        "content_base64",
        "label",
        "expected_revision",
        "limit",
        "after",
        "brief_text",
        "documents",
        "policy",
        "budget",
        "items",
        "sequence",
        "preset",
        "expires_in_seconds",
        "enabled",
        "description",
        "project_id",
        "__proto__",
        "constructor",
        "",
    ];
    let n = rng.below(6);
    let fields: Vec<String> = (0..n)
        .map(|_| {
            let k = if rng.chance(6) {
                weird_segment(rng)
            } else {
                (*rng.pick(KEYS)).to_owned()
            };
            format!(
                "{}:{}",
                serde_json::Value::String(k),
                json_value(rng, depth)
            )
        })
        .collect();
    format!("{{{}}}", fields.join(","))
}

fn mutate(rng: &mut Prng, mut b: Vec<u8>) -> Vec<u8> {
    for _ in 0..rng.below(4) {
        if b.is_empty() {
            break;
        }
        let i = rng.below(b.len());
        match rng.below(4) {
            0 => b[i] ^= 1 << rng.below(8),
            1 => {
                b.remove(i);
            }
            2 => b.insert(i, rng.next_u64() as u8),
            _ => b.truncate(i),
        }
    }
    b
}

/// Uma requisição HTTP malformada (ou quase) como bytes.
fn gen_request(rng: &mut Prng, port: u16, admin: &str, pid: &str) -> Vec<u8> {
    let ops = catalog::ops();
    let op = &ops[rng.below(ops.len())];
    let method = match rng.below(12) {
        0 => (*rng.pick(&[
            "HEAD", "TRACE", "CONNECT", "OPTIONS", "PUT", "PATCH", "PROPFIND", "GET",
        ]))
        .to_owned(),
        1 => String::from_utf8_lossy(
            &{
                let n = 1 + rng.below(10);
                rng.bytes(n)
            }
            .iter()
            .map(|b| b'A' + b % 26)
            .collect::<Vec<_>>(),
        )
        .into_owned(),
        _ => op.method.to_owned(),
    };
    let mut target = fill(op.path, rng, pid);
    if rng.chance(4) {
        let n = rng.below(5);
        target.push('?');
        for _ in 0..n {
            target.push_str(&format!("{}={}&", weird_segment(rng), weird_segment(rng)));
        }
    }
    if rng.chance(10) {
        target = mutate(rng, target.into_bytes())
            .iter()
            .map(|b| {
                if *b < 0x20 || *b > 0x7e {
                    '%'
                } else {
                    *b as char
                }
            })
            .collect();
    }
    if rng.chance(25) {
        target = match rng.below(5) {
            0 => format!("http://127.0.0.1{target}"),
            1 => "*".into(),
            2 => format!("//{}", target.trim_start_matches('/')),
            3 => target.replace('/', "%2f"),
            _ => String::new(),
        };
    }
    let version = if rng.chance(12) {
        *rng.pick(&["HTTP/1.0", "HTTP/2.0", "HTTP/0.9", "HTTP/1.", "http/1.1", ""])
    } else {
        "HTTP/1.1"
    };
    let mut body: Vec<u8> = match rng.below(6) {
        0 => Vec::new(),
        1 => json_object(rng, 0).into_bytes(),
        2 => {
            let b = json_object(rng, 0).into_bytes();
            mutate(rng, b)
        }
        3 => {
            let n = rng.below(600);
            rng.bytes(n)
        }
        4 => format!(
            "{}{}",
            "[".repeat(rng.below(200)),
            "]".repeat(rng.below(200))
        )
        .into_bytes(),
        _ => json_object(rng, 0).into_bytes(),
    };
    if rng.chance(25) {
        body.clear();
    }
    let mut head = format!("{method} {target} {version}\r\n");
    // Host
    match rng.below(10) {
        0 => {}
        1 => head.push_str(&format!("Host: {}\r\n", weird_segment(rng))),
        2 => head.push_str(&format!("Host: 127.0.0.1:{port}\r\nHost: evil\r\n")),
        _ => head.push_str(&format!("Host: 127.0.0.1:{port}\r\n")),
    }
    // Authorization
    match rng.below(10) {
        0 => {}
        1 => head.push_str(&format!("Authorization: Bearer {}\r\n", weird_segment(rng))),
        2 => head.push_str(&format!("Authorization: {admin}\r\n")),
        3 => head.push_str(&format!(
            "Authorization: Bearer {admin}\r\nAuthorization: Bearer {admin}\r\n"
        )),
        _ => head.push_str(&format!("Authorization: Bearer {admin}\r\n")),
    }
    // Content-Length / Content-Type
    match rng.below(10) {
        0 => {}
        1 => head.push_str("Content-Length: -1\r\n"),
        2 => head.push_str(&format!(
            "Content-Length: {}\r\n",
            body.len() + 1 + rng.below(100)
        )),
        3 => head.push_str(&format!(
            "Content-Length: {}\r\n",
            body.len().saturating_sub(1)
        )),
        4 => head.push_str("Transfer-Encoding: chunked\r\n"),
        _ => head.push_str(&format!("Content-Length: {}\r\n", body.len())),
    }
    match rng.below(6) {
        0 => {}
        1 => head.push_str("Content-Type: text/plain\r\n"),
        2 => head.push_str(&format!("Content-Type: {}\r\n", weird_segment(rng))),
        _ => head.push_str("Content-Type: application/json\r\n"),
    }
    // extras: Origin, Idempotency-Key, X-Request-Id, X-Capia-Filename, X-Capia-Sha256, lixo
    if rng.chance(8) {
        head.push_str(&format!("Origin: {}\r\n", weird_segment(rng)));
    }
    if rng.chance(5) {
        head.push_str(&format!("Idempotency-Key: {}\r\n", weird_segment(rng)));
    }
    if rng.chance(5) {
        head.push_str(&format!("X-Request-Id: {}\r\n", weird_segment(rng)));
    }
    if rng.chance(5) {
        head.push_str(&format!(
            "X-Capia-Filename: {}\r\nX-Capia-Sha256: {}\r\n",
            weird_segment(rng),
            weird_segment(rng)
        ));
    }
    for _ in 0..rng.below(4) {
        if rng.chance(3) {
            head.push_str(&format!(
                "X-{}: {}\r\n",
                weird_segment(rng).replace(['%', '/', ' '], "z"),
                weird_segment(rng)
            ));
        }
    }
    if rng.chance(30) {
        head.push_str(&format!("X-Fuzz-{}: v\r\n", rng.next_u64()));
    }
    head.push_str("Connection: close\r\n\r\n");
    let mut out = head.into_bytes();
    out.extend_from_slice(&body);
    if rng.chance(15) {
        out = mutate(rng, out);
    }
    out
}

struct Verdict {
    status: Option<u16>,
    elapsed: Duration,
}

fn run_case(s: &TestServer, bytes: &[u8], bound: Duration) -> Result<Verdict, String> {
    let t0 = Instant::now();
    let out =
        exchange_half_close(s.addr, bytes, bound).ok_or("could not connect (server down?)")?;
    let elapsed = t0.elapsed();
    if elapsed >= bound {
        return Err(format!("no answer or close within {bound:?}"));
    }
    let Some(r) = try_parse(&out) else {
        return Ok(Verdict {
            status: None,
            elapsed,
        });
    };
    if r.status >= 500 && !DOCUMENTED_5XX.contains(&r.status) {
        return Err(format!(
            "undocumented {}: {}",
            r.status,
            String::from_utf8_lossy(&r.body)
        ));
    }
    // 2xx pode legitimamente carregar o segredo único de um token recém-criado
    if r.status >= 400
        && let Some(why) = leak_in_unless_echo(&whole(&r), s.dir.path(), bytes)
    {
        return Err(format!("leak ({why}) in: {}", whole(&r)));
    }
    Ok(Verdict {
        status: Some(r.status),
        elapsed,
    })
}

fn save_crasher(bytes: &[u8], port: u16, admin: &str, tag: &str) {
    // só grava se pedido (CAPIA_FUZZ_SAVE=1): o corpus é revisado por humano antes de commitar
    if std::env::var_os("CAPIA_FUZZ_SAVE").is_none() {
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fuzz-corpus");
    let _ = std::fs::create_dir_all(&dir);
    let text = String::from_utf8_lossy(bytes)
        .replace(&format!("127.0.0.1:{port}"), "{{HOST}}")
        .replace(admin, "{{TOKEN}}");
    let _ = std::fs::write(
        dir.join(format!("crash-{tag}.http")),
        text.replace("\r\n", "\n"),
    );
}

#[test]
fn seeded_fuzzing_never_hangs_never_500s_and_leaves_the_server_healthy() {
    let cases = env_num("CAPIA_FUZZ_CASES", 400);
    let seed = env_num("CAPIA_FUZZ_SEED", 0xF022_5EED);
    let s = start("fuzz", |c| {
        c.rate_scale = 10_000.0;
        c.workers = 6;
        c.queue = 128;
        c.max_json_bytes = 64 * 1024;
        c.max_upload_bytes = 64 * 1024;
        c.upload_quota_bytes = 1 << 20;
    });
    let pid = s.create_project("fuzz");
    let (t0, f0) = (threads(), fds());
    let mut rng = Prng::new(seed);
    let mut by_status = std::collections::BTreeMap::<String, usize>::new();
    let mut slowest = Duration::ZERO;
    for i in 0..cases {
        let case_seed = rng.0;
        let bytes = gen_request(&mut rng, s.addr.port(), &s.admin, &pid);
        match run_case(&s, &bytes, Duration::from_secs(12)) {
            Ok(v) => {
                *by_status
                    .entry(v.status.map_or("closed".into(), |s| s.to_string()))
                    .or_default() += 1;
                slowest = slowest.max(v.elapsed);
            }
            Err(why) => {
                save_crasher(&bytes, s.addr.port(), &s.admin, &format!("{seed:x}-{i}"));
                panic!(
                    "fuzz case #{i} (seed {seed:#x}, state {case_seed:#x}) broke an invariant: {why}\n--- request ---\n{}",
                    String::from_utf8_lossy(&bytes).replace(&s.admin, "<TOKEN>")
                );
            }
        }
        // o servidor está vivo a cada 50 casos (cedo é melhor que tarde)
        if i % 50 == 49 {
            assert!(
                healthy(&s),
                "the server stopped being healthy after case #{i}"
            );
        }
    }
    assert!(
        healthy(&s),
        "unhealthy after {cases} fuzz cases; statuses {by_status:?}"
    );
    // um pedido válido, de verdade, depois de tudo
    assert_eq!(s.call("GET", "/v1/projects", None).status, 200);
    // limites de recursos
    assert!(
        wait_until(Duration::from_secs(10), || threads() <= t0 + 16),
        "threads grew {t0} → {}",
        threads()
    );
    assert!(fds() <= f0 + 64, "fds grew {f0} → {}", fds());
    eprintln!("fuzz: {cases} cases, statuses {by_status:?}, slowest {slowest:?}");
}

// ---- corpus commitado -----------------------------------------------------------------------------

/// `\n` → `\r\n` só no cabeçalho; `{{CL}}` = tamanho do corpo; `{{HOST}}`/`{{TOKEN}}` substituídos.
fn corpus_request(raw: &str, host: &str, token: &str) -> Vec<u8> {
    let raw = raw.replace("\r\n", "\n");
    let (head, body) = raw.split_once("\n\n").unwrap_or((raw.as_str(), ""));
    let body = body.trim_end_matches('\n');
    let head = head
        .replace("{{HOST}}", host)
        .replace("{{TOKEN}}", token)
        .replace("{{CL}}", &body.len().to_string())
        .replace('\n', "\r\n");
    let mut out = format!("{head}\r\n\r\n").into_bytes();
    out.extend_from_slice(body.replace("{{HOST}}", host).as_bytes());
    out
}

#[test]
fn the_committed_fuzz_corpus_replays_cleanly() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fuzz-corpus");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/fuzz-corpus must exist")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "http"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 12,
        "the corpus must not be emptied ({} files)",
        files.len()
    );
    let s = start("fuzz-corpus", |c| {
        c.rate_scale = 10_000.0;
        c.max_json_bytes = 64 * 1024;
    });
    let host = format!("127.0.0.1:{}", s.addr.port());
    for f in &files {
        let raw = std::fs::read_to_string(f).unwrap();
        let bytes = corpus_request(&raw, &host, &s.admin);
        if let Err(why) = run_case(&s, &bytes, Duration::from_secs(12)) {
            panic!("corpus entry {} broke an invariant: {why}", f.display());
        }
    }
    assert!(healthy(&s));
}
