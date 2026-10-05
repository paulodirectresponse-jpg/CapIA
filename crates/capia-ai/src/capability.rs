//! Capabilities com **origem** (Declared | Probed | Preset): o Router prefere evidência `Probed`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    TextGeneration,
    ToolCalling,
    ParallelToolCalls,
    StructuredOutput,
    Streaming,
    VisionInput,
    AudioInput,
    VideoInput,
    PdfInput,
    SpeechToText,
    TextToSpeech,
    Embeddings,
    // só conhecidas (Fase 4 não constrói autonomia de geração)
    ImageGeneration,
    VideoGeneration,
}

impl Capability {
    pub const ALL: [Capability; 14] = [
        Self::TextGeneration,
        Self::ToolCalling,
        Self::ParallelToolCalls,
        Self::StructuredOutput,
        Self::Streaming,
        Self::VisionInput,
        Self::AudioInput,
        Self::VideoInput,
        Self::PdfInput,
        Self::SpeechToText,
        Self::TextToSpeech,
        Self::Embeddings,
        Self::ImageGeneration,
        Self::VideoGeneration,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Dito pelo usuário/provider, não verificado.
    Declared,
    /// Sugestão de preset (data da informação).
    Preset,
    /// Verificado por probe real (ou replay de probe).
    Probed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityEntry {
    pub supported: bool,
    pub origin: Origin,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Capabilities {
    #[serde(default)]
    entries: BTreeMap<Capability, CapabilityEntry>,
}

impl Capabilities {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, c: Capability, supported: bool, origin: Origin) -> &mut Self {
        // evidência `Probed` nunca é rebaixada por uma declaração posterior
        if let Some(e) = self.entries.get(&c)
            && e.origin == Origin::Probed
            && origin != Origin::Probed
        {
            return self;
        }
        self.entries
            .insert(c, CapabilityEntry { supported, origin });
        self
    }

    pub fn declared(list: &[Capability]) -> Self {
        let mut c = Self::new();
        for k in list {
            c.set(*k, true, Origin::Declared);
        }
        c
    }

    pub fn preset(list: &[Capability]) -> Self {
        let mut c = Self::new();
        for k in list {
            c.set(*k, true, Origin::Preset);
        }
        c
    }

    pub fn get(&self, c: Capability) -> Option<&CapabilityEntry> {
        self.entries.get(&c)
    }

    /// Suportada (qualquer origem), exceto se um probe disse que **não**.
    pub fn has(&self, c: Capability) -> bool {
        self.entries.get(&c).is_some_and(|e| e.supported)
    }

    /// Suportada com evidência de probe.
    pub fn verified(&self, c: Capability) -> bool {
        self.entries
            .get(&c)
            .is_some_and(|e| e.supported && e.origin == Origin::Probed)
    }

    pub fn origin(&self, c: Capability) -> Option<Origin> {
        self.entries.get(&c).map(|e| e.origin)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Capability, &CapabilityEntry)> {
        self.entries.iter()
    }

    /// Aplica o resultado de um probe (sobrescreve a origem para `Probed`).
    pub fn apply_probe(&mut self, verified: &[Capability], failed: &[Capability]) {
        for c in verified {
            self.entries.insert(
                *c,
                CapabilityEntry {
                    supported: true,
                    origin: Origin::Probed,
                },
            );
        }
        for c in failed {
            self.entries.insert(
                *c,
                CapabilityEntry {
                    supported: false,
                    origin: Origin::Probed,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn probed_evidence_is_not_downgraded_by_declaration() {
        let mut c = Capabilities::declared(&[Capability::VisionInput]);
        assert_eq!(c.origin(Capability::VisionInput), Some(Origin::Declared));
        c.apply_probe(&[], &[Capability::VisionInput]);
        assert!(!c.has(Capability::VisionInput));
        c.set(Capability::VisionInput, true, Origin::Declared);
        assert!(!c.has(Capability::VisionInput), "probe vence a declaração");
        assert!(!c.verified(Capability::VisionInput));
    }

    #[test]
    fn serde_roundtrip() {
        let mut c = Capabilities::preset(&[Capability::TextGeneration, Capability::Streaming]);
        c.apply_probe(&[Capability::ToolCalling], &[]);
        let j = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Capabilities>(&j).unwrap(), c);
    }
}
