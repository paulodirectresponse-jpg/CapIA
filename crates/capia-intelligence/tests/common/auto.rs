//! Mundo de teste da autonomia: projeto real + mídia real + brain Replay roteirizado por papel +
//! Orchestrator real. O "modelo" é um respondedor determinístico: reconhece o papel pelo prompt de
//! sistema (`ROLE: producer|planner|critic`) e devolve JSON válido; assim a Run completa roda sem
//! rede e com resultado reproduzível.
#![allow(dead_code, unreachable_pub, clippy::unwrap_used, clippy::expect_used)]

use super::*;
use capia_ai::providers::replay::{ReplayProvider, ReplayResponse};
use capia_ai::types::{ChatEvent, ChatRequest};
use capia_intelligence::autonomy::gateway::GatewayRegistry;
use capia_intelligence::autonomy::generation::GenerationRegistry;
use capia_intelligence::autonomy::machine::RunStatus;
use capia_intelligence::autonomy::model::{AiRun, RunInputs, RunPolicy};
use capia_intelligence::autonomy::orchestrator::{Deps, Orchestrator};
use capia_intelligence::engine::{Engine, SessionEngine};
use capia_intelligence::error::IntelResult;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub fn chat_json(v: &Value) -> ReplayResponse {
    ReplayResponse::Chat {
        events: vec![ChatEvent::TextDelta {
            text: v.to_string(),
        }],
        chunk_delay_ms: 0,
    }
}

pub fn system_of(req: &ChatRequest) -> String {
    req.messages
        .first()
        .map(|m| m.text_of())
        .unwrap_or_default()
}

pub fn user_of(req: &ChatRequest) -> String {
    req.messages
        .iter()
        .skip(1)
        .map(|m| m.text_of())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Roteiro do "modelo". Cada campo é uma função do estado do teste.
#[allow(clippy::type_complexity)]
pub struct Script {
    pub demand: Box<dyn Fn() -> Value + Send + Sync>,
    pub producer: Box<dyn Fn() -> Value + Send + Sync>,
    /// `(deliverable_key, nº da chamada deste deliverable)`.
    pub planner: Box<dyn Fn(&str, u32) -> Value + Send + Sync>,
    /// Nº do review (0, 1, …).
    pub critic: Box<dyn Fn(u32) -> Value + Send + Sync>,
    pub counts: Mutex<std::collections::BTreeMap<String, u32>>,
    /// Todos os prompts enviados ao "modelo": `(sistema, usuário)`.
    pub prompts: Mutex<Vec<(String, String)>>,
    /// "Modelo de visão" do Replay: recebe os quadros que o Critic enviou (PNG decodificado).
    #[allow(clippy::type_complexity)]
    pub vision: Mutex<Option<VisionOracle>>,
    /// Quantas imagens chegaram ao modelo em cada chamada de Critic.
    pub images_seen: Mutex<Vec<usize>>,
    /// Atraso (ms) entre os dois pedaços da resposta do Producer (para testar pausa/cancelamento).
    pub producer_delay_ms: std::sync::atomic::AtomicU64,
    /// Faz a primeira chamada do Producer falhar com 429 (retry/fallback).
    pub rate_limit_first_producer: std::sync::atomic::AtomicBool,
}

impl Script {
    pub fn new(
        demand: impl Fn() -> Value + Send + Sync + 'static,
        producer: impl Fn() -> Value + Send + Sync + 'static,
        planner: impl Fn(&str, u32) -> Value + Send + Sync + 'static,
        critic: impl Fn(u32) -> Value + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            demand: Box::new(demand),
            producer: Box::new(producer),
            planner: Box::new(planner),
            critic: Box::new(critic),
            counts: Mutex::new(Default::default()),
            prompts: Mutex::new(Vec::new()),
            vision: Mutex::new(None),
            images_seen: Mutex::new(Vec::new()),
            producer_delay_ms: Default::default(),
            rate_limit_first_producer: Default::default(),
        })
    }

    pub fn bump(&self, k: &str) -> u32 {
        let mut c = self.counts.lock().unwrap();
        let e = c.entry(k.to_owned()).or_insert(0);
        let n = *e;
        *e += 1;
        n
    }

    pub fn count(&self, k: &str) -> u32 {
        self.counts.lock().unwrap().get(k).copied().unwrap_or(0)
    }
}

pub fn deliverable_of(user: &str) -> String {
    let marker = "\"deliverable_key\":\"";
    user.find(marker)
        .map(|i| {
            let rest = &user[i + marker.len()..];
            rest[..rest.find('"').unwrap_or(0)].to_owned()
        })
        .unwrap_or_default()
}

pub fn brain_for(script: Arc<Script>) -> Arc<ReplayProvider> {
    Arc::new(ReplayProvider::responder(
        "brain",
        Box::new(move |req, _n| {
            let sys = system_of(req);
            script
                .prompts
                .lock()
                .unwrap()
                .push((sys.clone(), user_of(req)));
            if sys.starts_with("ROLE: producer") {
                let n = script.bump("producer");
                if n == 0 && script.rate_limit_first_producer.load(Ordering::SeqCst) {
                    return ReplayResponse::Error {
                        code: capia_ai::ErrorCode::RateLimited,
                        message: "slow down".into(),
                        status: Some(429),
                        retry_after_ms: Some(10),
                        after_events: vec![],
                    };
                }
                let v = (script.producer)().to_string();
                let delay = script.producer_delay_ms.load(Ordering::SeqCst);
                if delay > 0 {
                    let (a, b) = v.split_at(v.len() / 2);
                    return ReplayResponse::Chat {
                        events: vec![
                            ChatEvent::TextDelta { text: a.into() },
                            ChatEvent::TextDelta { text: b.into() },
                        ],
                        chunk_delay_ms: delay,
                    };
                }
                chat_json(&serde_json::from_str::<Value>(&v).unwrap())
            } else if sys.starts_with("ROLE: planner") {
                let d = deliverable_of(&user_of(req));
                let n = script.bump(&format!("planner:{d}"));
                chat_json(&(script.planner)(&d, n))
            } else if sys.starts_with("ROLE: critic") {
                let n = script.bump("critic");
                let seen = seen_frames(req);
                script.images_seen.lock().unwrap().push(seen.len());
                let oracle = script.vision.lock().unwrap().clone();
                match (oracle, seen.is_empty()) {
                    (Some(o), false) => chat_json(&o(&seen)),
                    _ => chat_json(&(script.critic)(n)),
                }
            } else {
                script.bump("demand");
                chat_json(&(script.demand)())
            }
        }),
    ))
}

pub fn demand_json(extra_open: bool) -> Value {
    let nul = || json!({"value": null, "basis": "inferred", "sources": []});
    let f = |v: &str, u: &str| json!({"value": v, "basis": "explicit", "sources": [{"doc": "D1", "unit": u, "quote": v}]});
    json!({
        "title": "Cafe Serra Azul",
        "product": f("Cafe Serra Azul", "u1"),
        "audience": nul(), "offer": nul(), "objective": nul(), "tone": nul(), "platform": nul(),
        "duration_and_format": nul(),
        "cta": f("Compre agora", "u2"),
        "key_claims": [], "must_include": [], "must_avoid": [], "constraints": [], "assets_mentioned": [],
        "open_questions": if extra_open { json!([{"question": "Qual a duracao alvo?", "reason": "nao consta"}]) } else { json!([]) }
    })
}

pub fn producer_json(deliverables: Value, needs: Value) -> Value {
    json!({"strategy": "single master from the raw footage", "deliverables": deliverables, "asset_needs": needs,
           "constraints": [], "assumptions": ["raw footage covers the script"], "risks": []})
}

pub fn edit_json(asset: &str, extra_beats: Value) -> Value {
    let mut beats = vec![
        json!({"id": "hook", "role": "hook", "duration_ms": 3000, "asset": {"asset_id": asset, "source_in_ms": 0},
               "overlays": [{"text": "Voce precisa ver isso", "start_offset_ms": 0, "duration_ms": 1500}],
               "captions": [{"text": "Ola pessoal bem-vindos", "start_offset_ms": 0, "duration_ms": 1800}]}),
        json!({"id": "cta", "role": "cta", "duration_ms": 2000, "asset": {"asset_id": asset, "source_in_ms": 7000},
               "overlays": [{"text": "Compre agora", "start_offset_ms": 0, "duration_ms": 2000}]}),
    ];
    if let Some(a) = extra_beats.as_array() {
        beats.extend(a.iter().cloned());
    }
    json!({"format": {"width": 1080, "height": 1920, "fps": 30}, "beats": beats, "estimated_duration_ms": 5000,
           "global": {}, "constraints_checked": ["cta"]})
}

/// Engine que registra a ordem das chamadas de escrita (prova de "nada antes de validar").
pub struct SpyEngine {
    pub inner: SessionEngine,
    pub log: Mutex<Vec<(String, u64)>>,
    pub previews: AtomicU32,
    pub applies: AtomicU32,
    /// Quantos quadros o Critic pediu ao compositor.
    pub frames: AtomicU32,
    /// Simula um engine sem compositor (`render_frame` → `None`).
    pub no_frames: std::sync::atomic::AtomicBool,
    /// Gancho chamado ANTES de cada preview com o nº dele (simula edição manual no meio da Run).
    #[allow(clippy::type_complexity)]
    pub on_preview: Mutex<Option<Box<dyn Fn(u32) + Send + Sync>>>,
}

impl core::fmt::Debug for SpyEngine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SpyEngine")
    }
}

fn ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

impl Engine for SpyEngine {
    fn read(&self, m: &str, p: Value) -> IntelResult<Value> {
        self.inner.read(m, p)
    }
    fn preview(&self, a: &capia_commands::Actor, l: &str, c: Value) -> IntelResult<Value> {
        let n = self.previews.fetch_add(1, Ordering::SeqCst);
        if let Some(h) = self.on_preview.lock().unwrap().as_ref() {
            h(n);
        }
        self.log.lock().unwrap().push(("preview".into(), ms()));
        self.inner.preview(a, l, c)
    }
    fn apply(&self, a: &capia_commands::Actor, t: &str) -> IntelResult<Value> {
        self.applies.fetch_add(1, Ordering::SeqCst);
        self.log.lock().unwrap().push(("apply".into(), ms()));
        self.inner.apply(a, t)
    }
    fn project_path(&self) -> Option<std::path::PathBuf> {
        self.inner.project_path()
    }
    fn toolchain(&self) -> Option<capia_media::MediaToolchain> {
        self.inner.toolchain()
    }
    fn revision(&self) -> IntelResult<u64> {
        self.inner.revision()
    }
    fn render_frame(
        &self,
        s: &str,
        at: i64,
        w: u32,
        h: u32,
    ) -> IntelResult<Option<capia_intelligence::engine::RawFrame>> {
        self.frames.fetch_add(1, Ordering::SeqCst);
        if self.no_frames.load(Ordering::SeqCst) {
            return Ok(None);
        }
        self.inner.render_frame(s, at, w, h)
    }
    fn import_begin(&self, p: &Path) -> IntelResult<String> {
        self.inner.import_begin(p)
    }
    fn import_poll(&self, t: &str) -> IntelResult<Value> {
        self.inner.import_poll(t)
    }
    fn import_cancel(&self, t: &str) -> IntelResult<bool> {
        self.inner.import_cancel(t)
    }
}

pub struct AutoWorld {
    pub w: World,
    pub script: Arc<Script>,
    pub orch: Arc<Orchestrator>,
    pub spy: Arc<SpyEngine>,
    pub gateways: Arc<GatewayRegistry>,
    pub generators: Arc<GenerationRegistry>,
    pub events: Arc<Mutex<Vec<Value>>>,
}

pub fn auto_policy() -> RunPolicy {
    RunPolicy {
        demand_spec: capia_intelligence::autonomy::model::SpecApproval::Auto,
        plan: capia_intelligence::autonomy::model::PlanApproval::Auto,
        ..RunPolicy::default()
    }
}

pub fn simple_script(asset: Arc<Mutex<String>>) -> Arc<Script> {
    Script::new(
        || demand_json(false),
        || {
            producer_json(
                json!([{"key": "main", "sequence_strategy": "standalone"}]),
                json!([]),
            )
        },
        move |_d, _n| edit_json(&asset.lock().unwrap(), json!([])),
        |_n| json!({"findings": []}),
    )
}

/// Pequeno vídeo REAL (probe/import de verdade) usado como B-roll "baixado"/"gerado".
pub fn broll_bytes(tc: &capia_media::MediaToolchain, dir: &Path, name: &str, secs: u32) -> Vec<u8> {
    let out = dir.join(name);
    let st = std::process::Command::new(tc.ffmpeg.as_ref().unwrap())
        .args([
            "-v",
            "error",
            "-y",
            "-nostdin",
            "-f",
            "lavfi",
            "-t",
            &secs.to_string(),
            "-i",
            "smptebars=size=320x180:rate=30",
        ])
        .args([
            "-f",
            "lavfi",
            "-t",
            &secs.to_string(),
            "-i",
            "sine=frequency=500:sample_rate=16000",
        ])
        .args([
            "-c:v",
            "mpeg4",
            "-q:v",
            "4",
            "-c:a",
            "aac",
            "-pix_fmt",
            "yuv420p",
            "-shortest",
        ])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    std::fs::read(out).unwrap()
}

pub fn auto_world(
    name: &str,
    script_for: impl FnOnce(Arc<Mutex<String>>) -> Arc<Script>,
) -> Option<AutoWorld> {
    auto_world_at(name, None, script_for)
}

/// `reopen = Some(dir)`: reabre o projeto deixado por um processo morto (kill real).
pub fn auto_world_at(
    name: &str,
    reopen: Option<std::path::PathBuf>,
    script_for: impl FnOnce(Arc<Mutex<String>>) -> Arc<Script>,
) -> Option<AutoWorld> {
    let asset_cell = Arc::new(Mutex::new(String::new()));
    let script = script_for(asset_cell.clone());
    let w = world_full_at(
        name,
        reopen,
        Some(make_speech_clip),
        vec![transcript_response()],
        true,
        Some(brain_for(script.clone())),
    )?;
    *asset_cell.lock().unwrap() = w.asset_id.clone();
    let spy = Arc::new(SpyEngine {
        inner: SessionEngine::new(w.session.clone()),
        log: Mutex::new(Vec::new()),
        previews: AtomicU32::new(0),
        applies: AtomicU32::new(0),
        frames: AtomicU32::new(0),
        no_frames: std::sync::atomic::AtomicBool::new(false),
        on_preview: Mutex::new(None),
    });
    let events: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let ev2 = events.clone();
    let gateways = Arc::new(GatewayRegistry::new());
    let generators = Arc::new(GenerationRegistry::new());
    let deps = Deps {
        engine: spy.clone(),
        ai: w.ctx.ai.clone(),
        gateways: gateways.clone(),
        generators: generators.clone(),
        app_db: None,
        sink: Arc::new(move |v| ev2.lock().unwrap().push(v)),
        rt: tokio::runtime::Handle::current(),
    };
    let proj = w.dir.join("p.capia");
    let orch = Orchestrator::open(deps, proj).unwrap();
    Some(AutoWorld {
        w,
        script,
        orch,
        spy,
        gateways,
        generators,
        events,
    })
}

impl AutoWorld {
    /// Simula reabrir o app: uma instância **nova** do Orchestrator sobre o mesmo projeto
    /// (nenhum estado em memória é herdado; só o que está no `.capia`).
    pub fn restart(&self) -> Arc<Orchestrator> {
        let ev = self.events.clone();
        let deps = Deps {
            engine: self.spy.clone(),
            ai: self.w.ctx.ai.clone(),
            gateways: self.gateways.clone(),
            generators: self.generators.clone(),
            app_db: None,
            sink: Arc::new(move |v| ev.lock().unwrap().push(v)),
            rt: tokio::runtime::Handle::current(),
        };
        Orchestrator::open(deps, self.w.dir.join("p.capia")).unwrap()
    }

    pub fn inputs(&self) -> RunInputs {
        RunInputs {
            brief_text: Some("Produto: Cafe Serra Azul.\n\nCTA: Compre agora.".into()),
            assets: vec![self.w.asset_id.clone()],
            deliverables: vec![capia_intelligence::autonomy::model::DeliverableRequest {
                key: "main".into(),
                max_duration_s: Some(30),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    pub fn create(&self, inputs: RunInputs, policy: RunPolicy) -> AiRun {
        self.orch
            .create_run(inputs, Some(policy), None, "pf", None)
            .unwrap()
    }

    /// Dirige a Run até parar (espera/terminal) e devolve o estado final.
    pub async fn run_to_rest(&self, id: &str) -> AiRun {
        self.orch.start(id).ok();
        self.wait(id, |r| {
            r.status != RunStatus::Running && r.status != RunStatus::Pending
        })
        .await
    }

    pub async fn wait(&self, id: &str, pred: impl Fn(&AiRun) -> bool) -> AiRun {
        let t0 = std::time::Instant::now();
        loop {
            let r = self.orch.load(id).unwrap();
            if pred(&r) && !self.orch.is_driving(id) {
                return r;
            }
            if t0.elapsed() >= std::time::Duration::from_secs(90) {
                let r2 = self.orch.load(id).unwrap();
                eprintln!("LAST_REVIEW {}", r2.checkpoint["last_review"]["findings"]);
                for st in self.orch.store().list_stages(id).unwrap() {
                    eprintln!("STAGE {} {} {} {}", st.seq, st.stage, st.status, st.json);
                }
            }
            assert!(
                t0.elapsed() < std::time::Duration::from_secs(90),
                "timeout waiting: {:?} {:?} pending={:?} error={:?} replan={:?} validation={:?}",
                r.status,
                r.stage,
                r.pending.as_ref().map(|p| (&p.kind, &p.question)),
                r.error,
                r.checkpoint["replan"],
                r.validation.as_ref().map(|v| &v.errors)
            );
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    pub fn stages_visited(&self, id: &str) -> Vec<String> {
        self.orch
            .store()
            .list_stages(id)
            .unwrap()
            .into_iter()
            .filter(|s| s.status == "completed")
            .map(|s| s.stage)
            .collect()
    }

    pub fn history_actors(&self) -> Vec<String> {
        let h = self.w.ctx.engine.read("history.list", json!({})).unwrap();
        h["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["actor"]["id"].as_str().unwrap().to_owned())
            .collect()
    }
}

/// Um quadro como o "modelo" o enxerga: rótulo + pixels (RGBA8).
#[derive(Clone, Debug)]
pub struct SeenFrame {
    pub index: usize,
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Extrai (rótulo, imagem) dos pedaços da mensagem de usuário, na ordem em que foram enviados.
pub fn seen_frames(req: &ChatRequest) -> Vec<SeenFrame> {
    use base64::Engine as _;
    use capia_ai::types::Part;
    let mut out = Vec::new();
    let mut label = String::new();
    for m in &req.messages {
        for p in &m.parts {
            match p {
                Part::Text { text } if text.starts_with("FRAME ") => label.clone_from(text),
                Part::Image { data_b64, .. } => {
                    let png = base64::engine::general_purpose::STANDARD
                        .decode(data_b64)
                        .unwrap();
                    let f = capia_intelligence::autonomy::vision::decode_png(&png).unwrap();
                    out.push(SeenFrame {
                        index: out.len(),
                        label: std::mem::take(&mut label),
                        width: f.width,
                        height: f.height,
                        rgba: f.rgba,
                    });
                }
                _ => {}
            }
        }
    }
    out
}

impl SeenFrame {
    fn px(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2]]
    }

    /// Fração de pixels dominantemente vermelhos entre os que NÃO são barra preta (o clipe 16:9
    /// é encaixado numa timeline vertical, então há faixas pretas em cima e embaixo).
    pub fn red_fraction(&self) -> f64 {
        let (mut red, mut content) = (0u32, 0u32);
        for y in 0..self.height {
            for x in 0..self.width {
                let [r, g, b] = self.px(x, y);
                if r > 24 || g > 24 || b > 24 {
                    content += 1;
                    if r > 150 && g < 90 && b < 90 {
                        red += 1;
                    }
                }
            }
        }
        if content == 0 {
            0.0
        } else {
            f64::from(red) / f64::from(content)
        }
    }

    /// Centroide (0..1) dos pixels quase brancos (o "sujeito" das fixtures), se houver.
    pub fn subject_centroid(&self) -> Option<(f64, f64)> {
        let (mut sx, mut sy, mut n) = (0f64, 0f64, 0f64);
        for y in 0..self.height {
            for x in 0..self.width {
                let [r, g, b] = self.px(x, y);
                if r > 220 && g > 220 && b > 220 {
                    sx += f64::from(x);
                    sy += f64::from(y);
                    n += 1.0;
                }
            }
        }
        (n > 20.0).then(|| {
            (
                sx / n / f64::from(self.width),
                sy / n / f64::from(self.height),
            )
        })
    }
}

/// "Modelo de visão" do Replay: olha os PIXELS de cada quadro. Regras fixas e conhecidas pelos
/// testes — (1) vermelho sólido = B-roll sem relação com o produto; (2) sujeito (branco) fora do
/// centro = enquadramento incorreto. O índice citado é o do quadro recebido.
/// Oráculo de visão do Replay.
pub type VisionOracle = Arc<dyn Fn(&[SeenFrame]) -> Value + Send + Sync>;

pub fn pixel_oracle() -> VisionOracle {
    Arc::new(|frames| {
        let mut findings = Vec::new();
        for f in frames {
            if f.red_fraction() > 0.8 {
                findings.push(json!({"key": format!("broll-wrong-{}", f.index), "severity": "major",
                    "category": "broll_fit", "expected": "B-roll that matches the product and the spoken line",
                    "observed": "the frame shows an unrelated solid red image", "confidence": 0.9,
                    "evidence": [{"kind": "frame", "detail": f.index.to_string()}]}));
            } else if let Some((cx, cy)) = f.subject_centroid()
                && (!(0.3..=0.7).contains(&cx) || !(0.25..=0.75).contains(&cy))
            {
                findings.push(json!({"key": format!("framing-{}", f.index), "severity": "major",
                    "category": "framing", "expected": "subject centered in the frame",
                    "observed": format!("the subject sits at ({cx:.2}, {cy:.2}), off-center"), "confidence": 0.85,
                    "evidence": [{"kind": "frame", "detail": f.index.to_string()}]}));
            }
        }
        json!({"summary": "visual review", "findings": findings})
    })
}

#[derive(Clone, Copy, Debug)]
pub enum ClipKind {
    /// Fundo verde, sujeito branco centralizado.
    Centered,
    /// Vermelho sólido (B-roll sem relação).
    Red,
    /// Fundo escuro, sujeito branco no canto superior esquerdo.
    OffCenter,
}

/// Gera um clipe 320x180@30 (+ áudio) com o conteúdo visual de `kind`.
pub fn color_clip(
    tc: &capia_media::MediaToolchain,
    dir: &Path,
    name: &str,
    secs: u32,
    kind: ClipKind,
) -> std::path::PathBuf {
    let out = dir.join(name);
    let vf = match kind {
        ClipKind::Centered => {
            "color=c=green:s=320x180:r=30,drawbox=x=110:y=45:w=100:h=90:color=white:t=fill"
        }
        ClipKind::Red => "color=c=red:s=320x180:r=30",
        ClipKind::OffCenter => {
            "color=c=0x202020:s=320x180:r=30,drawbox=x=10:y=8:w=70:h=50:color=white:t=fill"
        }
    };
    let st = std::process::Command::new(tc.ffmpeg.as_ref().unwrap())
        .args([
            "-v",
            "error",
            "-y",
            "-nostdin",
            "-f",
            "lavfi",
            "-t",
            &secs.to_string(),
            "-i",
            vf,
        ])
        .args([
            "-f",
            "lavfi",
            "-t",
            &secs.to_string(),
            "-i",
            "sine=frequency=500:sample_rate=16000",
        ])
        .args([
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
            "-c:a",
            "aac",
            "-pix_fmt",
            "yuv420p",
            "-shortest",
        ])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    out
}

impl AutoWorld {
    /// Importa um arquivo de mídia pelo caminho normal do projeto e devolve o `asset_id`.
    pub fn import_clip(&self, path: &Path) -> String {
        call(
            &self.w.session,
            "assets.import",
            json!({"paths": [path.display().to_string()]}),
        );
        let t0 = std::time::Instant::now();
        loop {
            let evs = call(&self.w.session, "events.poll", json!({}));
            if let Some(a) = evs["events"].as_array().and_then(|a| {
                a.iter()
                    .find(|e| {
                        e["kind"] == "import_finalized"
                            && e["result"]["path"].as_str().is_none_or(|p| {
                                p.ends_with(path.file_name().unwrap().to_str().unwrap())
                            })
                    })
                    .map(|e| e["result"]["asset_id"].as_str().unwrap().to_owned())
            }) {
                return a;
            }
            assert!(t0.elapsed().as_secs() < 60, "import timeout");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// Liga/desliga `VisionInput` no modelo do "cérebro" (o Capability Router decide a partir disso).
    pub fn set_vision(&self, on: bool) {
        use capia_ai::capability::{Capabilities, Capability};
        self.w.ctx.ai.update_registry(|r| {
            let mut caps = vec![
                Capability::TextGeneration,
                Capability::StructuredOutput,
                Capability::ToolCalling,
                Capability::Streaming,
            ];
            if on {
                caps.push(Capability::VisionInput);
            }
            if let Some(m) = r.models.get_mut("brain:m") {
                m.capabilities = Capabilities::declared(&caps);
            }
        });
    }
}
