//! Brain Profile, política de privacidade e requisitos mínimos do Brain (AI_PROVIDERS §2/§3).

use crate::capability::Capability;
use crate::registry::{ModelEndpoint, ProviderConfig};
use crate::usage::Budgets;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Janela mínima do Brain (spec atual).
pub const MIN_BRAIN_CONTEXT: u32 = 64_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "policy", content = "endpoint", rename_all = "snake_case")]
pub enum CapabilityPolicy {
    Auto,
    Model(String),
}

/// Classe de dado que sairia da máquina.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataClass {
    Text,
    DocumentText,
    /// Quadros amostrados e reduzidos.
    Frames,
    /// Áudio extraído e comprimido.
    Audio,
    /// Vídeo original/bruto.
    Video,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyPolicy {
    #[serde(default = "yes")]
    pub never_upload_video: bool,
    #[serde(default = "yes")]
    pub vision_frames_only: bool,
    #[serde(default)]
    pub transcription_local_only: bool,
    #[serde(default)]
    pub no_cloud_audio: bool,
    #[serde(default)]
    pub no_external_document_upload: bool,
}

fn yes() -> bool {
    true
}

impl Default for PrivacyPolicy {
    fn default() -> Self {
        Self {
            never_upload_video: true,
            vision_frames_only: true,
            transcription_local_only: false,
            no_cloud_audio: false,
            no_external_document_upload: false,
        }
    }
}

impl PrivacyPolicy {
    /// Os dados `class` podem ir para um destino `local` (máquina do usuário) ou nuvem?
    /// Local nunca é bloqueado. `Err` traz o motivo mostrável ao usuário.
    pub fn check(
        &self,
        local: bool,
        class: DataClass,
        capability: Capability,
    ) -> Result<(), String> {
        if local {
            return Ok(());
        }
        match class {
            DataClass::Video if self.never_upload_video || self.vision_frames_only => {
                Err("policy: original video is never uploaded (only sampled frames)".into())
            }
            DataClass::Audio if self.no_cloud_audio => {
                Err("policy: audio is not sent to cloud providers".into())
            }
            DataClass::Audio
                if self.transcription_local_only && capability == Capability::SpeechToText =>
            {
                Err("policy: transcription must be local".into())
            }
            DataClass::DocumentText if self.no_external_document_upload => {
                Err("policy: documents are not sent to external providers".into())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrainProfile {
    pub id: String,
    pub name: String,
    /// `ModelEndpoint.id` do PRIMARY AI BRAIN.
    pub brain: String,
    #[serde(default)]
    pub role_overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub capability_overrides: BTreeMap<Capability, CapabilityPolicy>,
    #[serde(default)]
    pub fallbacks: BTreeMap<Capability, Vec<String>>,
    #[serde(default)]
    pub budgets: Budgets,
    #[serde(default)]
    pub privacy: PrivacyPolicy,
}

impl BrainProfile {
    pub fn new(id: impl Into<String>, name: impl Into<String>, brain: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            brain: brain.into(),
            role_overrides: BTreeMap::new(),
            capability_overrides: BTreeMap::new(),
            fallbacks: BTreeMap::new(),
            budgets: Budgets::default(),
            privacy: PrivacyPolicy::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrainRejection {
    pub reason: String,
}

/// Requisitos do Brain: `text + tools + structured output` (ou emulação) e contexto ≥ 64k.
/// Devolve **todos** os motivos (a UI explica por que o modelo não pode ser Brain).
pub fn brain_rejections(
    m: &ModelEndpoint,
    provider: Option<&ProviderConfig>,
) -> Vec<BrainRejection> {
    let mut out = Vec::new();
    let mut r = |s: &str| out.push(BrainRejection { reason: s.into() });
    match provider {
        None => r("provider not found"),
        Some(p) if !p.enabled => r("provider is disabled"),
        _ => {}
    }
    if !m.enabled {
        r("model is disabled");
    }
    if !m.has(Capability::TextGeneration) {
        r("missing capability: text generation");
    }
    if !m.has(Capability::ToolCalling) {
        r("missing capability: tool calling");
    }
    if !m.has(Capability::StructuredOutput) {
        r("missing capability: structured output (or validated emulation)");
    }
    if m.context_window < MIN_BRAIN_CONTEXT {
        out.push(BrainRejection {
            reason: format!(
                "context window {} is below the {MIN_BRAIN_CONTEXT} tokens required",
                m.context_window
            ),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::capability::Capabilities;
    use crate::registry::ProviderKind;

    #[test]
    fn privacy_matrix() {
        let p = PrivacyPolicy::default();
        assert!(
            p.check(false, DataClass::Video, Capability::VisionInput)
                .is_err()
        );
        assert!(
            p.check(true, DataClass::Video, Capability::VisionInput)
                .is_ok()
        );
        assert!(
            p.check(false, DataClass::Frames, Capability::VisionInput)
                .is_ok()
        );
        let p = PrivacyPolicy {
            transcription_local_only: true,
            ..PrivacyPolicy::default()
        };
        assert!(
            p.check(false, DataClass::Audio, Capability::SpeechToText)
                .is_err()
        );
        assert!(
            p.check(true, DataClass::Audio, Capability::SpeechToText)
                .is_ok()
        );
        let p = PrivacyPolicy {
            no_external_document_upload: true,
            ..PrivacyPolicy::default()
        };
        assert!(
            p.check(false, DataClass::DocumentText, Capability::TextGeneration)
                .is_err()
        );
        assert!(
            p.check(false, DataClass::Text, Capability::TextGeneration)
                .is_ok()
        );
    }

    #[test]
    fn brain_requirements_list_every_reason() {
        let mut prov = ProviderConfig::new("p", ProviderKind::Replay, "r");
        prov.enabled = true;
        let mut m = ModelEndpoint::new("p:m", "p", "m");
        m.capabilities = Capabilities::declared(&[Capability::TextGeneration]);
        m.context_window = 8_000;
        let rej = brain_rejections(&m, Some(&prov));
        assert_eq!(rej.len(), 3, "{rej:?}");
        m.capabilities = Capabilities::declared(&[
            Capability::TextGeneration,
            Capability::ToolCalling,
            Capability::StructuredOutput,
        ]);
        m.context_window = 200_000;
        assert!(brain_rejections(&m, Some(&prov)).is_empty());
        assert!(!brain_rejections(&m, None).is_empty());
    }
}
