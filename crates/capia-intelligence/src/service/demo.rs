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

fn chat(v: &Value) -> ReplayResponse {
    ReplayResponse::Chat {
        events: vec![ChatEvent::TextDelta {
            text: v.to_string(),
        }],
        chunk_delay_ms: 0,
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

fn demand() -> Value {
    let nul = || json!({"value": null, "basis": "inferred", "sources": []});
    let f = |v: &str, u: &str| json!({"value": v, "basis": "explicit", "sources": [{"doc": "D1", "unit": u, "quote": v}]});
    json!({
        "title": "Demo", "product": f("Demo", "u1"),
        "audience": nul(), "offer": nul(), "objective": nul(), "tone": nul(), "platform": nul(),
        "duration_and_format": nul(), "cta": nul(),
        "key_claims": [], "must_include": [], "must_avoid": [], "constraints": [], "assets_mentioned": [],
        "open_questions": []
    })
}

fn producer() -> Value {
    json!({"strategy": "single master from the raw footage",
           "deliverables": [{"key": "main", "sequence_strategy": "standalone"}], "asset_needs": [],
           "constraints": [], "assumptions": [], "risks": []})
}

fn edit(asset: &str) -> Value {
    json!({"format": {"width": 1080, "height": 1920, "fps": 30}, "estimated_duration_ms": 4000,
           "beats": [
             {"id": "hook", "role": "hook", "duration_ms": 2000, "asset": {"asset_id": asset, "source_in_ms": 0},
              "overlays": [{"text": "Veja isso", "start_offset_ms": 0, "duration_ms": 1500}]},
             {"id": "cta", "role": "cta", "duration_ms": 2000, "asset": {"asset_id": asset, "source_in_ms": 2000},
              "overlays": [{"text": "Compre agora", "start_offset_ms": 0, "duration_ms": 2000}]}],
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
                    chat(&producer())
                } else if sys_first.starts_with("ROLE: planner") {
                    chat(&edit(&first_asset(&text_of(req, 1))))
                } else if sys_first.starts_with("ROLE: critic") {
                    chat(&json!({"findings": []}))
                } else {
                    let _ = sys;
                    chat(&demand())
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
            ]);
            m.context_window = 200_000;
            m.enabled = true;
            reg.models.insert(m.id.clone(), m);
            reg.profiles
                .insert("demo".into(), BrainProfile::new("demo", "demo", "brain:m"));
            reg.active_profile = Some("demo".into());
        });
    }
}
