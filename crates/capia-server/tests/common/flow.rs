//! Fluxo externo canônico, escrito UMA vez sobre um `Client` abstrato (REST ou MCP): o mesmo roteiro
//! roda pelas duas superfícies e devolve um resumo SEMÂNTICO (nunca prosa) para comparação.
#![allow(dead_code, unreachable_pub, clippy::unwrap_used, clippy::expect_used)]

use super::receiver::Receiver;
use super::*;
use base64::Engine as _;
use capia_server::catalog;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct CallErr {
    pub status: u16,
    pub code: String,
    pub body: Value,
}

pub trait Client {
    fn label(&self) -> &'static str;
    fn call(&self, op: &str, args: Value) -> Result<Value, CallErr>;
    /// Sobe um arquivo (REST: streaming; MCP: inline base64) e devolve o `upload_id`.
    fn upload(&self, filename: &str, bytes: &[u8]) -> String;
    /// Chamada com chave de idempotência (REST: cabeçalho; MCP: argumento).
    fn call_idem(&self, op: &str, args: Value, key: &str) -> Result<Value, CallErr>;
}

impl dyn Client + '_ {
    pub fn ok(&self, op: &str, args: Value) -> Value {
        self.call(op, args)
            .unwrap_or_else(|e| panic!("[{}] {op} failed: {e:?}", self.label()))
    }
}

// ---- REST ---------------------------------------------------------------------------------------

pub struct RestClient<'a> {
    pub s: &'a TestServer,
    pub token: String,
}

fn query_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

impl RestClient<'_> {
    fn go(&self, op: &str, args: &Value, idem: Option<&str>) -> Result<Value, CallErr> {
        let def = catalog::find(op).unwrap();
        let mut path = def.path.to_owned();
        let mut rest = args.as_object().cloned().unwrap_or_default();
        for seg in def.path.split('/') {
            if let Some(name) = seg.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
                let v = rest.remove(name).unwrap();
                path = path.replace(seg, &query_value(&v));
            }
        }
        let mut extra = vec![("Content-Type", "application/json")];
        if let Some(k) = idem {
            extra.push(("Idempotency-Key", k));
        }
        let (body, path) = if def.method == "GET" || def.method == "DELETE" {
            let q: Vec<String> = rest.iter().map(|(k, v)| format!("{k}={}", query_value(v))).collect();
            (None, if q.is_empty() { path } else { format!("{path}?{}", q.join("&")) })
        } else {
            (Some(Value::Object(rest).to_string().into_bytes()), path)
        };
        let r = request(self.s.addr, def.method, &path, Some(&self.token), &extra, body.as_deref());
        let json: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        if (200..300).contains(&r.status) {
            Ok(json)
        } else {
            Err(CallErr {
                status: r.status,
                code: json["code"].as_str().unwrap_or_default().to_owned(),
                body: json,
            })
        }
    }
}

impl Client for RestClient<'_> {
    fn label(&self) -> &'static str {
        "rest"
    }

    fn call(&self, op: &str, args: Value) -> Result<Value, CallErr> {
        self.go(op, &args, None)
    }

    fn call_idem(&self, op: &str, args: Value, key: &str) -> Result<Value, CallErr> {
        self.go(op, &args, Some(key))
    }

    fn upload(&self, filename: &str, bytes: &[u8]) -> String {
        let r = request(
            self.s.addr,
            "POST",
            "/v1/uploads",
            Some(&self.token),
            &[("Content-Type", "application/octet-stream"), ("X-Capia-Filename", filename)],
            Some(bytes),
        );
        assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
        let j = r.json();
        j["upload"]["upload_id"].as_str().or(j["upload_id"].as_str()).unwrap_or_else(|| panic!("upload response: {j}")).to_owned()
    }
}

// ---- MCP ----------------------------------------------------------------------------------------

pub struct McpClient<'a> {
    pub s: &'a TestServer,
    pub token: String,
}

impl McpClient<'_> {
    fn go(&self, op: &str, args: &Value, idem: Option<&str>) -> Result<Value, CallErr> {
        let name = catalog::find(op).unwrap().tool_name();
        let mut args = args.clone();
        if let Some(k) = idem {
            args["idempotency_key"] = json!(k);
        }
        let msg = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
        let r = request(
            self.s.addr,
            "POST",
            "/mcp",
            Some(&self.token),
            &[("Content-Type", "application/json")],
            Some(msg.to_string().as_bytes()),
        );
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let v = r.json();
        assert!(v.get("error").is_none(), "protocol error for {op}: {v}");
        let res = &v["result"];
        if res["isError"] == false {
            Ok(res["structuredContent"].clone())
        } else {
            Err(CallErr {
                status: res["_meta"]["x-capia-status"].as_u64().unwrap_or(0) as u16,
                code: res["structuredContent"]["code"].as_str().unwrap_or_default().to_owned(),
                body: res["structuredContent"].clone(),
            })
        }
    }
}

impl Client for McpClient<'_> {
    fn label(&self) -> &'static str {
        "mcp"
    }

    fn call(&self, op: &str, args: Value) -> Result<Value, CallErr> {
        self.go(op, &args, None)
    }

    fn call_idem(&self, op: &str, args: Value, key: &str) -> Result<Value, CallErr> {
        self.go(op, &args, Some(key))
    }

    fn upload(&self, filename: &str, bytes: &[u8]) -> String {
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        let v = self.go("uploads.create_inline", &json!({"filename": filename, "content_base64": b64}), None).unwrap();
        v["upload"]["upload_id"].as_str().unwrap().to_owned()
    }
}

// ---- utilidades ---------------------------------------------------------------------------------

pub fn fixture(name: &str) -> Vec<u8> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/media").join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("fixture {}: {e}", p.display()))
}

pub fn ffmpeg_available() -> bool {
    std::process::Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// `true` quando o teste deve prosseguir; sem FFmpeg falha se `CAPIA_REQUIRE_FFMPEG=1`.
pub fn ffmpeg_or_skip(test: &str) -> bool {
    if ffmpeg_available() {
        return true;
    }
    assert!(
        std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
        "{test}: FFmpeg/ffprobe is required (CAPIA_REQUIRE_FFMPEG=1)"
    );
    eprintln!("SKIP {test}: ffprobe not found (set CAPIA_REQUIRE_FFMPEG=1 to make this a failure)");
    false
}

pub fn poll<T>(what: &str, timeout: Duration, mut f: impl FnMut() -> Option<T>) -> T {
    let t0 = Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(t0.elapsed() < timeout, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Leva a Run a `completed` aprovando cada decisão pedida; devolve os tipos de decisão atendidos.
pub fn drive_run(c: &dyn Client, pid: &str, run_id: &str) -> (Value, Vec<String>) {
    let mut kinds: Vec<String> = Vec::new();
    let v = poll(&format!("run {run_id} to complete"), Duration::from_secs(180), || {
        let v = c.ok("runs.get", json!({"project_id": pid, "run_id": run_id}));
        match v["run"]["status"].as_str().unwrap_or_default() {
            "completed" => Some(v),
            "failed" | "cancelled" => panic!("[{}] run ended badly: {v}", c.label()),
            "waiting_user" => {
                let p = &v["run"]["pending"];
                if let (Some(id), Some(kind)) = (p["id"].as_str(), p["kind"].as_str()) {
                    c.ok(
                        "runs.approve",
                        json!({"project_id": pid, "run_id": run_id, "decision_id": id, "option": "approve"}),
                    );
                    kinds.push(kind.to_owned());
                }
                None
            }
            _ => None,
        }
    });
    (v, kinds)
}

/// Encoders aprovados e disponíveis de H.264 (decide se o export h264-mp4 pode rodar).
pub fn approved_h264_available(core: &capia_server::Core) -> bool {
    let caps = core.session_call("export.encoders", json!({})).unwrap();
    caps.as_array().into_iter().flatten().any(|e| {
        e["codec"] == "h264" && e["available"] == true && e["policy"] == "approved"
    })
}

/// Forma de uma sequence independente de ids aleatórios: trilhas, clips (tipo/tempo/texto).
pub fn sequence_shape(seq: &Value, run_id: &str) -> Value {
    let sid = seq["sequence"]["id"].as_str().or(seq["sequence"]["header"]["id"].as_str()).unwrap_or_default().to_owned();
    let strip = |s: &str| -> String {
        let s = s.replace(run_id, "RUN");
        let s = s.strip_prefix(&sid.replace(run_id, "RUN")).unwrap_or(&s).to_owned();
        s
    };
    let body = &seq["sequence"];
    let mut clips: Vec<Value> = body["clips"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(id, c)| {
            json!([
                strip(id),
                c["content"]["type"],
                c["start"],
                c["duration"],
                c["source_in"],
                c["track"].as_str().map(strip),
                c["content"]["text"],
            ])
        })
        .collect();
    clips.sort_by_key(Value::to_string);
    let mut tracks: Vec<Value> = body["tracks"]
        .as_object()
        .map(|m| m.iter().map(|(id, t)| json!([strip(id), t["kind"]])).collect())
        .or_else(|| {
            body["tracks"].as_array().map(|a| {
                a.iter().map(|t| json!([t["id"].as_str().map(strip), t["kind"]])).collect()
            })
        })
        .unwrap_or_default();
    tracks.sort_by_key(Value::to_string);
    json!({
        "clip_count": clips.len(),
        "tracks": tracks,
        "clips": clips,
        "frame_rate": body["header"]["frame_rate"],
        "width": body["header"]["width"],
        "height": body["header"]["height"],
    })
}

#[derive(Debug)]
pub struct FlowResult {
    pub project_id: String,
    pub master_run: String,
    pub variant_run: String,
    pub webhook_secret: String,
    pub webhook_id: String,
    /// Corpo cru de `sequences.get` (por id), exatamente como a superfície devolveu.
    pub sequences: Vec<(String, Value)>,
    /// Resumo semântico (comparável entre superfícies).
    pub summary: Value,
    pub export_ran: bool,
}

/// Run mínima (sem variantes/export): projeto → raw → import → Run → concluída. Devolve `(pid, run)`.
pub fn quick_run(c: &dyn Client, hook_url: Option<(&str, &[&str])>) -> (String, String) {
    if let Some((url, events)) = hook_url {
        c.ok("webhooks.create", json!({"url": url, "events": events}));
    }
    let pid = c.ok("projects.create", json!({"name": "quick"}))["project"]["id"].as_str().unwrap().to_owned();
    let up = c.upload("raw.mp4", &fixture("video_audio.mp4"));
    let t = c.ok("assets.import", json!({"project_id": pid, "upload_id": up}))["ticket_id"].as_str().unwrap().to_owned();
    poll("import", Duration::from_secs(60), || {
        (c.ok("imports.get", json!({"project_id": pid, "ticket_id": t}))["import"]["state"] == "finalized").then_some(())
    });
    let run = c.ok("runs.create", json!({"project_id": pid, "brief_text": "Produto: Demo. CTA: Compre agora."}))["run"]["id"].as_str().unwrap().to_owned();
    drive_run(c, &pid, &run);
    (pid, run)
}

/// O fluxo canônico: projeto → raw + referência + briefing → import → Run (aprovações) →
/// variantes → export → webhooks → leitura do estado.
pub fn run_flow(c: &dyn Client, s: &TestServer, rx: &Receiver) -> FlowResult {
    // webhook primeiro: tudo o que acontece depois é entregue
    let wh = c.ok(
        "webhooks.create",
        json!({"url": rx.url(), "events": ["run.completed", "export.completed", "run.failed", "export.failed"], "description": "canonical flow"}),
    );
    let (wh_id, secret) = (
        wh["webhook"]["id"].as_str().unwrap().to_owned(),
        wh["secret"].as_str().unwrap().to_owned(),
    );
    let pid = c.ok("projects.create", json!({"name": format!("flow-{}", c.label())}))["project"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let raw_up = c.upload("raw.mp4", &fixture("video_audio.mp4"));
    let ref_up = c.upload("reference.mp4", &fixture("video_only.mp4"));
    let brief_text = "Produto: Demo. CTA: Compre agora. [demo:needs-correction]";
    let doc_up = c.upload("brief.txt", format!("Briefing\n{brief_text}\nTom: direto.\n").as_bytes());
    // o upload do mesmo conteúdo duas vezes não duplica o asset
    let mut tickets = Vec::new();
    for up in [&raw_up, &ref_up] {
        let t = c.ok("assets.import", json!({"project_id": pid, "upload_id": up}));
        assert_eq!(t["state"], "queued");
        tickets.push(t["ticket_id"].as_str().unwrap().to_owned());
    }
    let mut asset_ids = Vec::new();
    for t in &tickets {
        let a = poll("asset import", Duration::from_secs(60), || {
            let v = c.ok("imports.get", json!({"project_id": pid, "ticket_id": t}));
            match v["import"]["state"].as_str().unwrap_or_default() {
                "finalized" => Some(v["import"]["asset_id"].as_str().unwrap().to_owned()),
                "failed" => panic!("import failed: {v}"),
                _ => None,
            }
        });
        asset_ids.push(a);
    }
    let assets = c.ok("assets.list", json!({"project_id": pid}));
    let n_assets = assets["assets"].as_array().unwrap().len();
    assert!(n_assets >= 1);
    let created = c.ok(
        "runs.create",
        json!({
            "project_id": pid, "brief_text": brief_text, "documents": [doc_up],
            "assets": [asset_ids[0]], "references": [asset_ids[1]],
        }),
    );
    let master = created["run"]["id"].as_str().unwrap().to_owned();
    let (master_view, master_kinds) = drive_run(c, &pid, &master);
    let stages: BTreeSet<String> = master_view["stages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["status"] == "completed")
        .filter_map(|s| s["stage"].as_str().map(str::to_owned))
        .collect();
    // variantes
    let before: BTreeSet<String> = c.ok("runs.list", json!({"project_id": pid}))["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect();
    c.ok("runs.variants", json!({"project_id": pid, "run_id": master, "count": 2}));
    let child = poll("variant run", Duration::from_secs(30), || {
        c.ok("runs.list", json!({"project_id": pid}))["runs"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r["id"].as_str())
            .find(|id| !before.contains(*id))
            .map(str::to_owned)
    });
    let (child_view, child_kinds) = drive_run(c, &pid, &child);
    // sequences (o estado autoritativo)
    let list = c.ok("sequences.list", json!({"project_id": pid}));
    let revision = list["revision"].clone();
    let mut sequences = Vec::new();
    for sq in list["sequences"].as_array().unwrap() {
        let id = sq["id"].as_str().unwrap().to_owned();
        let body = c.ok("sequences.get", json!({"project_id": pid, "sequence_id": id}));
        sequences.push((id, body));
    }
    let live = |view: &Value| -> Vec<Value> {
        view["run"]["sequences"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| s["role"] != "superseded")
            .cloned()
            .collect()
    };
    let master_seqs = live(&master_view);
    let child_seqs = live(&child_view);
    let shape_of = |run: &str, sid: &str| -> Value {
        let body = &sequences.iter().find(|(id, _)| id == sid).unwrap_or_else(|| panic!("sequence {sid} not listed")).1;
        sequence_shape(body, run)
    };
    let mut seq_summary: Vec<Value> = master_seqs
        .iter()
        .map(|s| (master.as_str(), s))
        .chain(child_seqs.iter().map(|s| (child.as_str(), s)))
        .map(|(run, s)| {
            json!({
                "run": if run == master { "master" } else { "variants" },
                "role": s["role"], "deliverable": s["deliverable"],
                "shape": shape_of(run, s["sequence_id"].as_str().unwrap()),
            })
        })
        .collect();
    seq_summary.sort_by_key(Value::to_string);
    // histórico: só atores (a IA escreve como `run:<id>`, nunca como humano)
    let hist = c.ok("history.list", json!({"project_id": pid, "limit": 500}));
    let mut history: Vec<String> = hist["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| format!("{}:{}", e["actor"]["kind"].as_str().unwrap_or("?"), e["label"].as_str().unwrap_or("")))
        .map(|x| x.replace(&master, "MASTER").replace(&child, "CHILD"))
        .collect();
    history.sort();
    // export
    let first_seq = master_seqs[0]["sequence_id"].as_str().unwrap().to_owned();
    let h264 = approved_h264_available(s.core());
    let preset = if h264 { "h264-mp4" } else { "intermediate" };
    if !h264 {
        eprintln!(
            "[{}] NOTE: no approved+available H.264 encoder in this environment; the h264-mp4 export step is SKIPPED (exercising the `intermediate` preset instead)",
            c.label()
        );
    }
    let started = c.ok(
        "exports.start",
        json!({"project_id": pid, "items": [{"sequence": first_seq, "preset": preset, "width": 540, "height": 960}]}),
    );
    let export_id = started["exports"][0]["id"].as_str().unwrap().to_owned();
    let export = poll("export", Duration::from_secs(240), || {
        let v = c.ok("exports.get", json!({"project_id": pid, "export_id": export_id}));
        match v["export"]["state"].as_str().unwrap_or_default() {
            "completed" => Some(v["export"].clone()),
            "failed" | "cancelled" => panic!("export ended badly: {v}"),
            _ => None,
        }
    });
    let deliverables = c.ok("deliverables.list", json!({"project_id": pid}));
    assert_eq!(deliverables["exports"].as_array().unwrap().len(), 1);
    // webhooks: run.completed (master + variantes) e export.completed
    let want = 3;
    let got = rx.wait_for(want, Duration::from_secs(30));
    let mut types: Vec<String> = got.iter().map(|r| r.json()["type"].as_str().unwrap().to_owned()).collect();
    types.sort();
    let report = &export["report"];
    // sonda independente do arquivo entregue (ffprobe direto, fora do servidor)
    let out_path = s.core().db.export_get(&export_id).unwrap().unwrap().path.unwrap();
    let probe = if std::path::Path::new(&out_path).is_dir() {
        // preset `intermediate`: pasta de quadros (sem contêiner de vídeo para sondar)
        json!({"codec": "intermediate-frames", "width": report["width"], "height": report["height"]})
    } else {
        probe_video(&out_path)
    };
    if h264 {
        assert_eq!(probe["codec"], "h264", "{probe}");
    }
    assert_eq!((probe["width"].as_u64(), probe["height"].as_u64()), (Some(540), Some(960)), "{probe}");
    let summary = json!({
        "assets": n_assets,
        "approvals": {"master": sorted(&master_kinds), "variants": sorted(&child_kinds)},
        "stages_completed": stages,
        "review_loops_ge_1": master_view["usage"]["review_loops"].as_u64().unwrap_or(0) >= 1,
        "sequences": seq_summary,
        "variant_sequences": child_seqs.len(),
        "history": history,
        "revision": revision,
        "export": {
            "preset": export["preset"], "state": export["state"],
            "has_report": !report.is_null(),
            "report_width": report["width"], "report_height": report["height"],
            "report_frames": report["frames"],
            "probe": probe,
        },
        "webhook_types": types,
        "run_actor_in_history": hist["entries"].as_array().unwrap().iter().any(|e| e["actor"]["id"].as_str().is_some_and(|a| a.starts_with("run:"))),
    });
    let normalise = |v: Value| -> Value {
        let mut t = v.to_string();
        t = t.replace(&master, "MASTER").replace(&child, "CHILD").replace(&pid, "PROJECT");
        for (i, a) in asset_ids.iter().enumerate() {
            t = t.replace(a.as_str(), &format!("ASSET{i}"));
        }
        serde_json::from_str(&t).unwrap()
    };
    let summary = normalise(summary);
    FlowResult {
        project_id: pid.clone(),
        master_run: master.clone(),
        variant_run: child.clone(),
        webhook_secret: secret,
        webhook_id: wh_id,
        sequences,
        summary,
        export_ran: true,
    }
}

fn sorted(v: &[String]) -> Vec<String> {
    let mut v = v.to_vec();
    v.sort();
    v.dedup();
    v
}

/// Codec/largura/altura do primeiro stream de vídeo, por ffprobe direto.
pub fn probe_video(path: &str) -> Value {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name,width,height", "-of", "json", path])
        .output()
        .expect("ffprobe");
    assert!(out.status.success(), "ffprobe failed on {path} (exists: {}): {}", std::path::Path::new(path).exists(), String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let st = &v["streams"][0];
    json!({"codec": st["codec_name"], "width": st["width"], "height": st["height"]})
}
