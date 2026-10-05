//! DEV/E2E (feature `testkit`): "cérebro" determinístico (Replay) que responde por papel
//! (`ROLE: producer|planner|critic`, senão DemandSpec). O produto não chama isto: sem ele um
//! provider `replay` falha com `NOT_CONFIGURED`. Permite o E2E de UI e o pipeline headless sem
//! rede nem chave.
use super::IntelligenceService;
use capia_ai::brain::BrainProfile;
use capia_ai::capability::{Capabilities, Capability};
use capia_ai::providers::replay::{ReplayProvider, ReplayResponse};
use capia_ai::registry::{ModelEndpoint, ProviderConfig, ProviderKind};
use capia_ai::types::{ChatEvent, ChatRequest};
use serde_json::{Value, json};
use std::sync::Arc;

/// `CAPIA_AI_DEMO_DELAY_MS`: atraso entre os dois pedaços da resposta (E2E de interrupção: a Run
/// fica `running` tempo suficiente para o teste matar o app no meio).
fn demo_delay_ms() -> u64 {
    std::env::var("CAPIA_AI_DEMO_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
        .min(10_000)
}

fn chat(v: &Value) -> ReplayResponse {
    let text = v.to_string();
    let delay = demo_delay_ms();
    if delay == 0 {
        return ReplayResponse::Chat {
            events: vec![ChatEvent::TextDelta { text }],
            chunk_delay_ms: 0,
        };
    }
    let mid = text.len() / 2;
    let mid = (mid..=text.len())
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(text.len());
    let (a, b) = text.split_at(mid);
    ReplayResponse::Chat {
        events: vec![
            ChatEvent::TextDelta { text: a.to_owned() },
            ChatEvent::TextDelta { text: b.to_owned() },
        ],
        chunk_delay_ms: delay,
    }
}

fn text_of(req: &ChatRequest, skip: usize) -> String {
    req.messages
        .iter()
        .skip(skip)
        .map(|m| m.text_of())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Primeiro `ast_…` citado no prompt (inventário de assets do projeto).
fn first_asset(prompt: &str) -> String {
    prompt
        .find("\"ast_")
        .map(|i| {
            let rest = &prompt[i + 1..];
            rest[..rest.find('"').unwrap_or(rest.len())].to_owned()
        })
        .unwrap_or_default()
}

/// Marcador do briefing que faz o cérebro demo errar de propósito (dispara REVIEW → CORRECT).
const NEEDS_CORRECTION: &str = "[demo:needs-correction]";

fn demand(brief: &str) -> Value {
    let nul = || json!({"value": null, "basis": "inferred", "sources": []});
    let f = |v: &str, u: &str| json!({"value": v, "basis": "explicit", "sources": [{"doc": "D1", "unit": u, "quote": v}]});
    // só afirma o CTA se o briefing realmente o contém (a fonte é verificada literalmente)
    let cta = if brief.contains("Compre agora") {
        f("Compre agora", "u1")
    } else {
        nul()
    };
    let title = if brief.contains(NEEDS_CORRECTION) {
        format!("Demo {NEEDS_CORRECTION}")
    } else {
        "Demo".to_owned()
    };
    json!({
        "title": title, "product": f("Demo", "u1"),
        "audience": nul(), "offer": nul(), "objective": nul(), "tone": nul(), "platform": nul(),
        "duration_and_format": nul(), "cta": cta,
        "key_claims": [], "must_include": [], "must_avoid": [], "constraints": [], "assets_mentioned": [],
        "open_questions": []
    })
}

fn producer(prompt: &str) -> Value {
    // pedido de variantes (`ai.run.variants`): duas variantes standalone; senão, o master
    let deliverables = if prompt.contains("\"variants\": {") {
        json!([{"key": "var1", "sequence_strategy": "standalone"},
               {"key": "var2", "sequence_strategy": "standalone"}])
    } else {
        json!([{"key": "main", "sequence_strategy": "standalone"}])
    };
    json!({"strategy": "single master from the raw footage",
           "deliverables": deliverables, "asset_needs": [],
           "constraints": [], "assumptions": [], "risks": []})
}

fn edit(asset: &str, prompt: &str) -> Value {
    let key = prompt
        .find("\"deliverable_key\":\"")
        .map(|i| {
            let rest = &prompt[i + 19..];
            rest[..rest.find('"').unwrap_or(0)].to_owned()
        })
        .unwrap_or_default();
    let hook = match key.as_str() {
        "var1" => "Variante 1",
        "var2" => "Variante 2",
        _ => "Veja isso",
    };
    let mut cta_beat = json!({"id": "cta", "role": "cta", "duration_ms": 400,
                              "asset": {"asset_id": asset, "source_in_ms": 400}});
    // erro proposital (só no briefing marcado): o CTA do briefing fica sem texto na tela
    if !prompt.contains(NEEDS_CORRECTION) {
        cta_beat["overlays"] =
            json!([{"text": "Compre agora", "start_offset_ms": 0, "duration_ms": 400}]);
    }
    // trechos curtos: cabem em qualquer fonte de ≥ 1 s (a fixture de E2E tem 1 s)
    json!({"format": {"width": 1080, "height": 1920, "fps": 30}, "estimated_duration_ms": 800,
           "beats": [
             {"id": "hook", "role": "hook", "duration_ms": 400, "asset": {"asset_id": asset, "source_in_ms": 0},
              "overlays": [{"text": hook, "start_offset_ms": 0, "duration_ms": 300}]},
             cta_beat],
           "global": {}, "constraints_checked": []})
}

impl IntelligenceService {
    /// Registra o cérebro de demonstração, um perfil `demo` ativo e o provider `brain` (Replay).
    pub fn install_demo_autonomy(&self) {
        let brain = Arc::new(ReplayProvider::responder(
            "brain",
            Box::new(|req, _n| {
                let sys = text_of(req, 0);
                let sys_first = req
                    .messages
                    .first()
                    .map(|m| m.text_of())
                    .unwrap_or_default();
                if sys_first.starts_with("ROLE: producer") {
                    chat(&producer(&text_of(req, 1)))
                } else if sys_first.starts_with("ROLE: planner") {
                    let user = text_of(req, 1);
                    chat(&edit(&first_asset(&user), &user))
                } else if sys_first.starts_with("ROLE: critic") {
                    chat(&json!({"findings": []}))
                } else {
                    let _ = sys;
                    chat(&demand(&text_of(req, 1)))
                }
            }),
        ));
        self.ai.register_replay("brain", brain);
        self.ai.update_registry(|reg| {
            let mut p = ProviderConfig::new("brain", ProviderKind::Replay, "brain");
            p.enabled = true;
            reg.providers.insert("brain".into(), p);
            let mut m = ModelEndpoint::new("brain:m", "brain", "brain-replay");
            m.capabilities = Capabilities::declared(&[
                Capability::TextGeneration,
                Capability::StructuredOutput,
                Capability::ToolCalling,
                Capability::Streaming,
                // o Critic também pode olhar quadros amostrados (Replay devolve "sem achados")
                Capability::VisionInput,
            ]);
            m.context_window = 200_000;
            m.enabled = true;
            reg.models.insert(m.id.clone(), m);
            reg.profiles
                .insert("demo".into(), BrainProfile::new("demo", "demo", "brain:m"));
            reg.active_profile = Some("demo".into());
            // o app de E2E pode ter herdado `ai_enabled=false` de um teste anterior (AppDb persistido)
            reg.ai_enabled = true;
        });
    }
}
