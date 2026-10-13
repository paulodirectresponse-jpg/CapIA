//! Ator de uma escrita no documento. Tipo **folha e neutro**: a inteligência (`capia-intelligence`)
//! e o `EnginePort` precisam dele sem depender do modelo/Command Engine Rust legado
//! (`capia-model`/`capia-commands`), que re-exporta estes tipos (API inalterada).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    User,
    Agent,
    Api,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Actor {
    pub kind: ActorKind,
    pub id: String,
}

impl Actor {
    pub fn new(kind: ActorKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }

    pub fn user(id: impl Into<String>) -> Self {
        Self::new(ActorKind::User, id)
    }

    pub fn agent(id: impl Into<String>) -> Self {
        Self::new(ActorKind::Agent, id)
    }

    pub fn api(id: impl Into<String>) -> Self {
        Self::new(ActorKind::Api, id)
    }

    pub fn system() -> Self {
        Self::new(ActorKind::System, "system")
    }

    /// `Agent` e `Api` só escrevem por `preview → apply_plan` (ADR-030).
    pub fn requires_preview(&self) -> bool {
        matches!(self.kind, ActorKind::Agent | ActorKind::Api)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn agents_and_api_require_preview() {
        assert!(Actor::agent("run:1").requires_preview());
        assert!(Actor::api("k").requires_preview());
        assert!(!Actor::user("u").requires_preview());
        assert!(!Actor::system().requires_preview());
    }

    #[test]
    fn serde_shape_is_stable() {
        let json = serde_json::to_string(&Actor::agent("a")).unwrap();
        assert_eq!(json, r#"{"kind":"agent","id":"a"}"#);
    }
}
