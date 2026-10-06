//! "Conectar IA": do provedor + chave ao Brain Profile ativo em uma operação só (RC2).
//! Importa os modelos, escolhe um padrão sensato, mede o que ele faz de verdade (probe por
//! capability) e cria o perfil `auto` — o usuário nunca fica com `active_profile = null`.

use crate::error::{IntelError, IntelResult};
use capia_ai::CancelToken;
use capia_ai::brain::{BrainProfile, brain_rejections};
use capia_ai::capability::{Capabilities, Capability};
use capia_ai::dispatcher::AiRuntime;
use serde_json::{Value, json};
use std::sync::Arc;

/// Contexto assumido quando o provedor não informa (o `/models` da OpenAI não informa).
const DEFAULT_CONTEXT: u32 = 128_000;

/// Escolhe o modelo de chat padrão entre os ids que o provedor lista: famílias recentes primeiro,
/// sem variantes de áudio/imagem/embedding, preferindo o alias curto (sem data) e o não-"mini".
pub(crate) fn pick_model(ids: &[String]) -> Option<String> {
    const FAMILIES: [&str; 4] = ["gpt-5", "gpt-4.1", "gpt-4o", "gpt-4"];
    const REJECT: [&str; 14] = [
        "audio",
        "realtime",
        "image",
        "tts",
        "transcribe",
        "embedding",
        "moderation",
        "instruct",
        "search",
        "codex",
        "whisper",
        "dall",
        "preview",
        "nano",
    ];
    ids.iter()
        .filter(|id| !REJECT.iter().any(|r| id.contains(r)))
        .filter_map(|id| {
            let fam = FAMILIES.iter().position(|f| id.starts_with(f))?;
            let dated = id.chars().filter(char::is_ascii_digit).count() > 6;
            Some(((fam, id.contains("mini"), dated, id.len()), id.clone()))
        })
        .min_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, id)| id)
}

pub(crate) async fn finish_connect(
    ai: Arc<AiRuntime>,
    provider_id: String,
    wanted: Option<String>,
    cancel: CancelToken,
    persist: impl Fn() + Send,
) -> IntelResult<Value> {
    // 1) conexão + autenticação: listar modelos é a prova mais barata
    ai.import_models(&provider_id, &cancel).await?;
    let ids: Vec<String> = ai
        .registry()
        .models
        .values()
        .filter(|m| m.provider_id == provider_id)
        .map(|m| m.model_id.clone())
        .collect();
    let model = wanted
        .filter(|w| ids.contains(w))
        .or_else(|| pick_model(&ids))
        .ok_or_else(|| {
            IntelError::new(
                "NO_SUITABLE_MODEL",
                "the provider did not list a chat model this app can use",
            )
        })?;
    let endpoint = format!("{provider_id}:{model}");
    // 2) habilita com capabilities DECLARADAS; o probe abaixo as confirma ou derruba
    ai.update_registry(|r| {
        r.ai_enabled = true;
        if let Some(m) = r.models.get_mut(&endpoint) {
            m.enabled = true;
            if m.context_window == 0 {
                m.context_window = DEFAULT_CONTEXT;
            }
            m.capabilities = Capabilities::declared(&[
                Capability::TextGeneration,
                Capability::Streaming,
                Capability::ToolCalling,
                Capability::StructuredOutput,
                Capability::VisionInput,
            ]);
        }
    });
    persist();
    // 3) probe por capability: o resultado diz a verdade, não o catálogo
    let probe = ai.probe_endpoint(&endpoint, &cancel).await?;
    // 4) Brain Profile automático, só se o modelo serve de Brain
    let (profile_created, reasons) = {
        let reg = ai.registry();
        let rej = reg
            .models
            .get(&endpoint)
            .map(|m| brain_rejections(m, reg.providers.get(&provider_id)))
            .unwrap_or_default();
        (
            rej.is_empty() && probe.connected,
            rej.into_iter().map(|r| r.reason).collect::<Vec<_>>(),
        )
    };
    if profile_created {
        ai.update_registry(|r| {
            let mut p = BrainProfile::new("auto", "Automático", endpoint.clone());
            if let Some(old) = r.profiles.get("auto") {
                p.budgets = old.budgets.clone();
                p.privacy = old.privacy.clone();
            }
            r.profiles.insert("auto".into(), p);
            r.active_profile = Some("auto".into());
        });
    }
    persist();
    Ok(json!({
        "provider_id": provider_id,
        "model": model,
        "endpoint_id": endpoint,
        "probe": probe,
        "profile_created": profile_created,
        "active_profile": ai.registry().active_profile,
        "brain_rejections": reasons,
    }))
}

#[cfg(test)]
mod tests {
    use super::pick_model;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn picks_newest_family_short_alias_not_mini_or_media() {
        let l = ids(&[
            "gpt-4o-mini",
            "gpt-4o",
            "gpt-5-mini",
            "gpt-5-2025-08-07",
            "gpt-5",
            "gpt-5-codex",
            "gpt-4o-audio-preview",
            "text-embedding-3-small",
            "whisper-1",
            "gpt-image-1",
        ]);
        assert_eq!(pick_model(&l).as_deref(), Some("gpt-5"));
    }

    #[test]
    fn falls_back_through_families_and_none_when_nothing_fits() {
        assert_eq!(
            pick_model(&ids(&["gpt-4o-mini", "gpt-4.1-mini"])).as_deref(),
            Some("gpt-4.1-mini")
        );
        assert_eq!(pick_model(&ids(&["whisper-1", "tts-1"])), None);
    }
}
