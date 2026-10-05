//! Mundo de teste: projeto real (Session), mídia real gerada pelo ffmpeg, provider Replay.
#![allow(dead_code, unreachable_pub, clippy::unwrap_used, clippy::expect_used)]

pub mod auto;

use capia_ai::brain::BrainProfile;
use capia_ai::capability::{Capabilities, Capability};
use capia_ai::dispatcher::{AiRuntime, MemoryCache, TaskCtx};
use capia_ai::providers::replay::{ReplayProvider, ReplayResponse};
use capia_ai::registry::{ModelEndpoint, ProviderConfig, ProviderKind, Registry};
use capia_ai::stt::{Segment, Transcript, Word};
use capia_ai::usage::MemorySink;
use capia_editor_api::{Reply, Session, SessionConfig};
use capia_intelligence::{IntelCtx, SessionEngine};
use capia_media::{MediaConfig, MediaToolchain};
use capia_secrets::MemoryStore;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const FRAME: i64 = 23_520_000; // 30 fps

pub fn ffmpeg() -> Option<MediaToolchain> {
    match MediaToolchain::locate(&MediaConfig::default()) {
        Ok(t) if t.ffmpeg.is_some() => Some(t),
        other => {
            assert!(
                std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
                "CAPIA_REQUIRE_FFMPEG=1 mas o ffmpeg não foi encontrado: {other:?}"
            );
            eprintln!("SKIP (sem ffmpeg)");
            None
        }
    }
}

pub fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("capia-intel-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 12 s, 320x180@30: fala (tom) em 0–2, 4,5–6,5 e 7–12 s; silêncio longo em 2–4,5 s e curto em 6,5–7 s.
pub fn make_speech_clip(tc: &MediaToolchain, dir: &Path) -> PathBuf {
    let out = dir.join("speech.mp4");
    let st = Command::new(tc.ffmpeg.as_ref().unwrap())
        .args(["-v", "error", "-y", "-nostdin"])
        .args([
            "-f",
            "lavfi",
            "-t",
            "12",
            "-i",
            "testsrc=size=320x180:rate=30",
        ])
        .args([
            "-f",
            "lavfi",
            "-t",
            "12",
            "-i",
            "aevalsrc=0.5*sin(2*PI*300*t)*(lt(t\\,2)+between(t\\,4.5\\,6.5)+gte(t\\,7)):s=16000",
        ])
        .args([
            "-c:v", "mpeg4", "-q:v", "3", "-c:a", "aac", "-pix_fmt", "yuv420p",
        ])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    out
}

pub struct World {
    pub dir: PathBuf,
    pub session: Arc<Mutex<Session>>,
    pub ctx: IntelCtx,
    pub asset_id: String,
    pub stt: Arc<ReplayProvider>,
    pub brain: Option<Arc<ReplayProvider>>,
    pub sink: Arc<MemorySink>,
}

fn word(t: &str, s: f64, e: f64) -> Word {
    Word {
        start_us: (s * 1e6) as i64,
        end_us: (e * 1e6) as i64,
        text: t.into(),
        confidence: Some(0.9),
    }
}

/// Transcrição roteirizada que casa com o áudio de [`make_speech_clip`].
pub fn scripted_transcript() -> Transcript {
    let words = vec![
        word("Olá", 0.1, 0.5),
        word("pessoal", 0.5, 1.0),
        word("bem-vindos.", 1.0, 1.9),
        word("Hoje", 4.6, 5.0),
        word("tem", 5.0, 5.3),
        word("oferta", 5.3, 6.4),
        word("Compre", 7.1, 7.8),
        word("agora", 7.8, 9.0),
        word("mesmo.", 9.0, 11.5),
    ];
    Transcript {
        schema_version: 1,
        language: Some("pt".into()),
        duration_us: Some(12_000_000),
        segments: vec![Segment {
            start_us: 100_000,
            end_us: 11_500_000,
            text: "Olá pessoal bem-vindos. Hoje tem oferta Compre agora mesmo.".into(),
            confidence: Some(0.9),
            speaker: None,
            words,
        }],
    }
}

pub fn call(s: &Arc<Mutex<Session>>, m: &str, p: Value) -> Value {
    match s
        .lock()
        .unwrap()
        .call(m, p)
        .unwrap_or_else(|e| panic!("{m}: {e}"))
    {
        Reply::Json(v) => v,
        Reply::Binary { .. } => panic!("json esperado"),
    }
}

pub fn world(name: &str, stt_script: Vec<ReplayResponse>) -> Option<World> {
    world_media(name, make_speech_clip, stt_script, true)
}

/// 9,6 s: 4 planos de 2,4 s (cortes secos em 2,4/4,8/7,2 s) + tom contínuo.
pub fn make_cut_clip(tc: &MediaToolchain, dir: &Path) -> PathBuf {
    let out = dir.join("cuts.mp4");
    let mut args: Vec<String> = ["-v", "error", "-y", "-nostdin"].map(String::from).to_vec();
    for src in ["testsrc", "smptebars", "rgbtestsrc", "yuvtestsrc"] {
        args.extend(["-f", "lavfi", "-t", "2.4", "-i"].map(String::from));
        args.push(format!("{src}=size=320x180:rate=30"));
    }
    args.extend(
        [
            "-f",
            "lavfi",
            "-t",
            "9.6",
            "-i",
            "sine=frequency=300:sample_rate=16000",
        ]
        .map(String::from),
    );
    args.extend([
        "-filter_complex".into(),
        "[0:v][1:v][2:v][3:v]concat=n=4:v=1:a=0[v]".into(),
        "-map".into(),
        "[v]".into(),
        "-map".into(),
        "4:a".into(),
    ]);
    args.extend(
        [
            "-c:v", "mpeg4", "-q:v", "3", "-c:a", "aac", "-pix_fmt", "yuv420p",
        ]
        .map(String::from),
    );
    args.push(out.display().to_string());
    let st = Command::new(tc.ffmpeg.as_ref().unwrap())
        .args(&args)
        .status()
        .unwrap();
    assert!(st.success());
    out
}

pub fn world_media(
    name: &str,
    make: fn(&MediaToolchain, &Path) -> PathBuf,
    stt_script: Vec<ReplayResponse>,
    with_stt: bool,
) -> Option<World> {
    world_full(name, Some(make), stt_script, with_stt, None)
}

/// Mundo completo: mídia opcional, STT opcional e um "brain" de texto (Replay) opcional.
pub fn world_full(
    name: &str,
    make: Option<fn(&MediaToolchain, &Path) -> PathBuf>,
    stt_script: Vec<ReplayResponse>,
    with_stt: bool,
    brain: Option<Arc<ReplayProvider>>,
) -> Option<World> {
    let tc = if make.is_some() {
        Some(ffmpeg()?)
    } else {
        None
    };
    let dir = tmp(name);
    let media = make.map(|m| m(tc.as_ref().unwrap(), &dir));
    let session = Arc::new(Mutex::new(Session::new(SessionConfig::default())));
    call(
        &session,
        "project.create",
        json!({ "path": dir.join("p.capia").display().to_string() }),
    );
    let mut asset_id = String::new();
    if let Some(media) = media {
        call(
            &session,
            "assets.import",
            json!({ "paths": [media.display().to_string()] }),
        );
        let deadline = Instant::now() + Duration::from_secs(60);
        asset_id = loop {
            let evs = call(&session, "events.poll", json!({}));
            if let Some(a) = evs["events"].as_array().and_then(|a| {
                a.iter()
                    .find(|e| e["kind"] == "import_finalized")
                    .map(|e| e["result"]["asset_id"].as_str().unwrap().to_owned())
            }) {
                break a;
            }
            assert!(Instant::now() < deadline, "import não terminou");
            std::thread::sleep(Duration::from_millis(50));
        };
        call(
            &session,
            "command.execute",
            json!({"label":"build","commands":[
                {"operation_id":"b1","type":"create_sequence","id":"s","name":"M","frame_rate":"30","width":1080,"height":1920},
                {"operation_id":"b2","type":"add_track","sequence":"s","id":"v","kind":"visual"},
                {"operation_id":"b3","type":"insert_clip","track":"v","start":0,
                 "clip":{"id":"c1","duration":FRAME*280,"content":{"type":"media","asset":asset_id,"has_video":true,"has_audio":true}}}
            ]}),
        );
    }

    let mut reg = Registry::new();
    if with_stt {
        let mut p = ProviderConfig::new("stt", ProviderKind::Replay, "stt");
        p.enabled = true;
        reg.providers.insert("stt".into(), p);
        let mut m = ModelEndpoint::new("stt:m", "stt", "whisper-replay");
        m.capabilities = Capabilities::declared(&[Capability::SpeechToText]);
        m.enabled = true;
        reg.models.insert(m.id.clone(), m);
    }
    let mut brain_replay = None;
    let mut brain_id = "stt:m".to_owned();
    if let Some(b) = brain {
        let mut p = ProviderConfig::new("brain", ProviderKind::Replay, "brain");
        p.enabled = true;
        reg.providers.insert("brain".into(), p);
        let mut m = ModelEndpoint::new("brain:m", "brain", "brain-replay");
        m.capabilities = Capabilities::declared(&[
            Capability::TextGeneration,
            Capability::StructuredOutput,
            Capability::ToolCalling,
            Capability::Streaming,
        ]);
        m.context_window = 200_000;
        m.enabled = true;
        reg.models.insert(m.id.clone(), m);
        brain_replay = Some(b);
        "brain:m".clone_into(&mut brain_id);
    }
    let prof = BrainProfile::new("pf", "pf", brain_id);
    reg.profiles.insert("pf".into(), prof.clone());
    reg.active_profile = Some("pf".into());
    let sink = Arc::new(MemorySink::default());
    let rt = Arc::new(
        AiRuntime::new(reg, Arc::new(MemoryStore::new()), sink.clone())
            .with_cache(Arc::new(MemoryCache::default())),
    );
    let stt = Arc::new(ReplayProvider::scripted("stt", stt_script));
    rt.register_replay("stt", stt.clone());
    if let Some(b) = &brain_replay {
        rt.register_replay("brain", b.clone());
    }
    let ctx = IntelCtx::new(Arc::new(SessionEngine::new(session.clone())), rt, prof);
    Some(World {
        dir,
        session,
        ctx,
        asset_id,
        stt,
        sink,
        brain: brain_replay,
    })
}

pub fn task(w: &World, id: &str) -> TaskCtx {
    TaskCtx::new(id, &w.ctx.profile)
}

pub fn transcript_response() -> ReplayResponse {
    ReplayResponse::Transcript {
        transcript: scripted_transcript(),
    }
}

pub fn scripted_brain(script: Vec<ReplayResponse>) -> Arc<ReplayProvider> {
    Arc::new(ReplayProvider::scripted("brain", script))
}

/// Serviço `ai.*` real sobre um projeto real (cofre em memória; AppDb em arquivo).
pub struct ServiceWorld {
    pub dir: PathBuf,
    pub session: Arc<Mutex<Session>>,
    pub svc: capia_intelligence::service::IntelligenceService,
    pub store: Arc<MemoryStore>,
    pub appdb: PathBuf,
    pub asset_id: String,
}

pub fn service_world(name: &str, with_media: bool) -> Option<ServiceWorld> {
    let mut asset_id = String::new();
    let dir;
    let session = Arc::new(Mutex::new(Session::new(SessionConfig::default())));
    if with_media {
        let tc = ffmpeg()?;
        dir = tmp(name);
        let media = make_speech_clip(&tc, &dir);
        call(
            &session,
            "project.create",
            json!({ "path": dir.join("p.capia").display().to_string() }),
        );
        call(
            &session,
            "assets.import",
            json!({ "paths": [media.display().to_string()] }),
        );
        let deadline = Instant::now() + Duration::from_secs(60);
        asset_id = loop {
            let evs = call(&session, "events.poll", json!({}));
            if let Some(a) = evs["events"].as_array().and_then(|a| {
                a.iter()
                    .find(|e| e["kind"] == "import_finalized")
                    .map(|e| e["result"]["asset_id"].as_str().unwrap().to_owned())
            }) {
                break a;
            }
            assert!(Instant::now() < deadline, "import não terminou");
            std::thread::sleep(Duration::from_millis(50));
        };
        call(
            &session,
            "command.execute",
            json!({"label":"build","commands":[
                {"operation_id":"b1","type":"create_sequence","id":"s","name":"M","frame_rate":"30","width":1080,"height":1920},
                {"operation_id":"b2","type":"add_track","sequence":"s","id":"v","kind":"visual"},
                {"operation_id":"b3","type":"insert_clip","track":"v","start":0,
                 "clip":{"id":"c1","duration":FRAME*280,"content":{"type":"media","asset":asset_id,"has_video":true,"has_audio":true}}}
            ]}),
        );
    } else {
        dir = tmp(name);
        call(
            &session,
            "project.create",
            json!({ "path": dir.join("p.capia").display().to_string() }),
        );
    }
    let store = Arc::new(MemoryStore::new());
    let appdb = dir.join("app.db");
    let svc = capia_intelligence::service::IntelligenceService::new(
        Arc::new(SessionEngine::new(session.clone())),
        capia_intelligence::service::ServiceConfig {
            appdb_path: Some(appdb.clone()),
            secrets: store.clone(),
        },
    )
    .unwrap();
    Some(ServiceWorld {
        dir,
        session,
        svc,
        store,
        appdb,
        asset_id,
    })
}

impl ServiceWorld {
    pub fn ai(&self, m: &str, p: Value) -> Value {
        self.svc.call(m, p).unwrap_or_else(|e| panic!("{m}: {e}"))
    }

    pub fn ai_err(&self, m: &str, p: Value) -> capia_intelligence::IntelError {
        self.svc.call(m, p).expect_err("erro esperado")
    }

    /// Espera o evento terminal da tarefa; devolve todos os eventos dela.
    pub fn wait_task(&self, task_id: &str, stop: &[&str]) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut mine: Vec<Value> = Vec::new();
        loop {
            for e in self.svc.poll_events() {
                if e["task_id"] == task_id {
                    mine.push(e);
                }
            }
            if mine
                .iter()
                .any(|e| stop.contains(&e["phase"].as_str().unwrap_or("")))
            {
                return mine;
            }
            assert!(
                Instant::now() < deadline,
                "tarefa {task_id} não terminou: {mine:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn save_provider(
        &self,
        id: &str,
        kind: &str,
        base_url: Option<&str>,
        key: Option<&str>,
    ) -> Value {
        let mut provider = json!({
            "id": id, "kind": kind, "display_name": id, "enabled": true,
            "allow_loopback": base_url.is_some(),
        });
        if let Some(u) = base_url {
            provider["base_url"] = json!(u);
        }
        let mut p = json!({ "provider": provider });
        if let Some(k) = key {
            p["api_key"] = json!(k);
        }
        self.ai("ai.provider.save", p)
    }

    pub fn save_model(&self, id: &str, provider: &str, model: &str, caps: &[&str]) -> Value {
        let m = capia_ai::registry::ModelEndpoint::new(id, provider, model);
        let mut v = serde_json::to_value(&m).unwrap();
        v["enabled"] = json!(true);
        v["context_window"] = json!(200_000);
        v["capabilities"] = serde_json::to_value(Capabilities::declared(
            &caps
                .iter()
                .map(|c| match *c {
                    "text" => Capability::TextGeneration,
                    "tools" => Capability::ToolCalling,
                    "stream" => Capability::Streaming,
                    "struct" => Capability::StructuredOutput,
                    "stt" => Capability::SpeechToText,
                    other => panic!("capability desconhecida {other}"),
                })
                .collect::<Vec<_>>(),
        ))
        .unwrap();
        self.ai("ai.model.save", json!({ "endpoint": v }))
    }

    /// Todos os bytes que o app escreveu em disco para este projeto + app db (para busca do canário).
    pub fn disk_bytes(&self) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        fn walk(d: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            if let Ok(rd) = std::fs::read_dir(d) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, out);
                    } else if let Ok(b) = std::fs::read(&p) {
                        out.push((p.display().to_string(), b));
                    }
                }
            }
        }
        walk(&self.dir, &mut out);
        out
    }
}
