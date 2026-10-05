//! Contexto de execução das pipelines: engine (leitura + gate de escrita), runtime de IA, perfil
//! ativo e registros do projeto.

use crate::engine::Engine;
use crate::error::IntelResult;
use crate::records::Records;
use capia_ai::brain::{BrainProfile, DataClass};
use capia_ai::capability::Capability;
use capia_ai::dispatcher::AiRuntime;
use capia_ai::router::RouteRequest;
use capia_commands::Actor;
use std::sync::Arc;

#[derive(Clone)]
pub struct IntelCtx {
    pub engine: Arc<dyn Engine>,
    pub ai: Arc<AiRuntime>,
    pub profile: BrainProfile,
    /// Ator das escritas (`Agent`): só `preview → apply_plan`.
    pub actor: Actor,
}

impl IntelCtx {
    pub fn new(engine: Arc<dyn Engine>, ai: Arc<AiRuntime>, profile: BrainProfile) -> Self {
        Self {
            engine,
            ai,
            profile,
            actor: Actor::agent("assistant"),
        }
    }

    pub fn records(&self) -> IntelResult<Records> {
        let path = self
            .engine
            .project_path()
            .ok_or_else(|| crate::error::IntelError::new("NO_PROJECT", "no project is open"))?;
        Records::open(&path)
    }

    pub fn stt_route(&self) -> RouteRequest {
        let mut r = RouteRequest::for_capability(Capability::SpeechToText);
        r.data.push(DataClass::Audio);
        r
    }

    pub fn is_local(&self, endpoint_id: &str) -> bool {
        self.ai.is_endpoint_local(endpoint_id)
    }
}

impl core::fmt::Debug for IntelCtx {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IntelCtx")
            .field("profile", &self.profile.id)
            .field("actor", &self.actor)
            .finish_non_exhaustive()
    }
}
