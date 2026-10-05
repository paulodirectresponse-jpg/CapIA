//! Capability Router: escolhe o endpoint por capability respeitando Brain Profile, overrides,
//! disponibilidade, evidência de probe, privacidade, contexto e orçamento — e **explica** a decisão.

use crate::brain::{BrainProfile, CapabilityPolicy, DataClass};
use crate::capability::Capability;
use crate::error::{ErrorCode, ProviderError};
use crate::registry::{Health, ModelEndpoint, Registry};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct RouteRequest {
    pub capability: Option<Capability>,
    /// Capabilities adicionais exigidas **juntas** (ex.: visão + saída estruturada).
    pub also_needs: Vec<Capability>,
    pub role: Option<String>,
    /// Dados que sairiam da máquina nesta chamada.
    pub data: Vec<DataClass>,
    /// Tokens de contexto necessários (estimativa), para descartar janelas pequenas.
    pub min_context: Option<u32>,
    /// Custo estimado **conhecido** (micro-unidades) para checar o orçamento por tarefa.
    pub est_cost_micros: Option<u64>,
    /// Exige evidência de probe (tarefas críticas).
    pub require_verified: bool,
    /// Endpoints a ignorar (já falharam nesta chamada lógica).
    pub exclude: Vec<String>,
}

impl RouteRequest {
    pub fn for_capability(c: Capability) -> Self {
        Self {
            capability: Some(c),
            ..Self::default()
        }
    }

    fn all_needs(&self) -> Vec<Capability> {
        let mut v: Vec<Capability> = self.capability.into_iter().collect();
        v.extend(self.also_needs.iter().copied());
        v.sort();
        v.dedup();
        v
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Considered {
    pub endpoint_id: String,
    pub rejected: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub endpoint_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub reason: String,
    pub considered: Vec<Considered>,
    pub requirements: Vec<Capability>,
    /// Havia evidência de probe para todas as capabilities pedidas?
    pub verified: bool,
}

/// Resolve **uma** rota. Erros explicam por que nenhuma serve (`NoCapableModel` /
/// `PrivacyPolicyBlocked` / `BudgetExceeded`).
pub fn resolve(
    reg: &Registry,
    profile: &BrainProfile,
    req: &RouteRequest,
) -> Result<Decision, ProviderError> {
    let all = chain(reg, profile, req)?;
    all.into_iter()
        .next()
        .ok_or_else(|| ProviderError::new(ErrorCode::NoCapableModel, "no capable model"))
}

/// Cadeia ordenada (primária + fallbacks compatíveis). Nunca vazia em `Ok`.
pub fn chain(
    reg: &Registry,
    profile: &BrainProfile,
    req: &RouteRequest,
) -> Result<Vec<Decision>, ProviderError> {
    if !reg.ai_enabled {
        return Err(ProviderError::new(
            ErrorCode::NotConfigured,
            "AI is turned off",
        ));
    }
    let needs = req.all_needs();
    let primary_cap = req.capability.unwrap_or(Capability::TextGeneration);
    let mut order: Vec<(String, &'static str)> = Vec::new();
    let mut push = |id: &str, why: &'static str| {
        if !order.iter().any(|(i, _)| i == id) {
            order.push((id.to_owned(), why));
        }
    };

    // 1) override por papel; 2) override por capability
    if let Some(role) = &req.role
        && let Some(id) = profile.role_overrides.get(role)
    {
        push(id, "role override");
    }
    if let Some(CapabilityPolicy::Model(id)) = profile.capability_overrides.get(&primary_cap) {
        push(id, "capability override");
    }
    // 3) o Brain (coerência), se tiver tudo
    push(&profile.brain, "brain");
    // 4) fallbacks configurados
    if let Some(list) = profile.fallbacks.get(&primary_cap) {
        for id in list {
            push(id, "configured fallback");
        }
    }
    // 5) qualquer outro modelo habilitado com a capability
    let mut extra: Vec<&ModelEndpoint> = reg.models.values().collect();
    extra.sort_by(|a, b| {
        let av = needs.iter().all(|c| a.capabilities.verified(*c));
        let bv = needs.iter().all(|c| b.capabilities.verified(*c));
        bv.cmp(&av)
            .then_with(|| price_rank(a).cmp(&price_rank(b)))
            .then_with(|| a.id.cmp(&b.id))
    });
    for m in extra {
        push(&m.id, "auto");
    }

    let mut considered = Vec::new();
    let mut ok: Vec<(Decision, Health)> = Vec::new();
    let mut blocked_privacy: Vec<String> = Vec::new();
    let mut blocked_budget = false;
    for (id, why) in &order {
        let Some(m) = reg.models.get(id) else {
            considered.push(Considered {
                endpoint_id: id.clone(),
                rejected: Some("unknown endpoint".into()),
            });
            continue;
        };
        match accept(reg, profile, m, req, &needs) {
            Ok(()) => {
                let verified = needs.iter().all(|c| m.capabilities.verified(*c));
                if req.require_verified && !verified {
                    considered.push(Considered {
                        endpoint_id: id.clone(),
                        rejected: Some("capability not verified by a probe".into()),
                    });
                    continue;
                }
                considered.push(Considered {
                    endpoint_id: id.clone(),
                    rejected: None,
                });
                ok.push((
                    Decision {
                        endpoint_id: m.id.clone(),
                        provider_id: m.provider_id.clone(),
                        model_id: m.model_id.clone(),
                        reason: (*why).to_owned(),
                        considered: Vec::new(),
                        requirements: needs.clone(),
                        verified,
                    },
                    m.health,
                ));
            }
            Err(Rej::Privacy(r)) => {
                blocked_privacy.push(r.clone());
                considered.push(Considered {
                    endpoint_id: id.clone(),
                    rejected: Some(r),
                });
            }
            Err(Rej::Budget(r)) => {
                blocked_budget = true;
                considered.push(Considered {
                    endpoint_id: id.clone(),
                    rejected: Some(r),
                });
            }
            Err(Rej::Other(r)) => considered.push(Considered {
                endpoint_id: id.clone(),
                rejected: Some(r),
            }),
        }
    }
    if ok.is_empty() {
        let (code, msg) = if !blocked_privacy.is_empty()
            && considered.iter().all(|c| c.rejected.is_some())
            && !blocked_budget
        {
            (
                ErrorCode::PrivacyPolicyBlocked,
                format!(
                    "no route satisfies the privacy policy: {}",
                    blocked_privacy.join("; ")
                ),
            )
        } else if blocked_budget {
            (
                ErrorCode::BudgetExceeded,
                "no route fits the task budget".to_owned(),
            )
        } else {
            (
                ErrorCode::NoCapableModel,
                format!("no enabled model has the required capabilities {needs:?}"),
            )
        };
        return Err(ProviderError::new(code, msg));
    }
    // Fallback **só** pelo que o usuário configurou (Brain, overrides, lista de fallbacks): se algum
    // endpoint nomeado serve, os "auto" ficam de fora — o prompt do usuário nunca vai a um provider
    // que ele não escolheu para esta capability. "Auto" só escolhe quando nada nomeado serve (ex.:
    // o Brain não tem STT/visão).
    if ok.iter().any(|(d, _)| d.reason != "auto") {
        ok.retain(|(d, _)| d.reason != "auto");
    }
    // estável: saudáveis antes de degradados, mantendo a ordem de prioridade
    ok.sort_by_key(|(_, h)| u8::from(*h == Health::Degraded));
    Ok(ok
        .into_iter()
        .map(|(mut d, _)| {
            d.considered = considered.clone();
            d
        })
        .collect())
}

fn price_rank(m: &ModelEndpoint) -> u64 {
    m.pricing.as_ref().map_or(u64::MAX, |p| {
        p.input_micros_per_mtok
            .saturating_add(p.output_micros_per_mtok)
    })
}

enum Rej {
    Privacy(String),
    Budget(String),
    Other(String),
}

fn accept(
    reg: &Registry,
    profile: &BrainProfile,
    m: &ModelEndpoint,
    req: &RouteRequest,
    needs: &[Capability],
) -> Result<(), Rej> {
    if req.exclude.iter().any(|e| e == &m.id) {
        return Err(Rej::Other("already failed in this call".into()));
    }
    if !reg.usable(m) {
        return Err(Rej::Other("model or provider disabled".into()));
    }
    if m.health == Health::Unavailable {
        return Err(Rej::Other("model marked unavailable".into()));
    }
    for c in needs {
        if !m.has(*c) {
            return Err(Rej::Other(format!("missing capability {c:?}")));
        }
    }
    if let Some(min) = req.min_context
        && m.context_window != 0
        && m.context_window < min
    {
        return Err(Rej::Other(format!(
            "context window {} < required {min}",
            m.context_window
        )));
    }
    let local = reg.provider_of(m).is_some_and(|p| p.kind.is_local());
    let cap = req.capability.unwrap_or(Capability::TextGeneration);
    for class in &req.data {
        profile
            .privacy
            .check(local, *class, cap)
            .map_err(Rej::Privacy)?;
    }
    if let (Some(est), Some(max)) = (
        req.est_cost_micros,
        profile.budgets.max_cost_per_task_micros,
    ) && est > max
    {
        return Err(Rej::Budget(format!(
            "estimated cost {est} exceeds the task budget {max}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::brain::PrivacyPolicy;
    use crate::capability::Capabilities;
    use crate::registry::{ProviderConfig, ProviderKind};

    fn world() -> (Registry, BrainProfile) {
        let mut r = Registry::new();
        for (id, kind) in [
            ("cloud", ProviderKind::OpenAiCompatible),
            ("local", ProviderKind::LocalOpenAiCompatible),
            ("stt", ProviderKind::WhisperLocal),
        ] {
            let mut p = ProviderConfig::new(id, kind, id);
            p.enabled = true;
            r.providers.insert(id.into(), p);
        }
        let full = [
            Capability::TextGeneration,
            Capability::ToolCalling,
            Capability::StructuredOutput,
            Capability::Streaming,
        ];
        let mut brain = ModelEndpoint::new("cloud:brain", "cloud", "brain");
        brain.capabilities = Capabilities::declared(&full);
        brain.context_window = 200_000;
        let mut vis = ModelEndpoint::new("cloud:vision", "cloud", "vision");
        vis.capabilities =
            Capabilities::declared(&[Capability::TextGeneration, Capability::VisionInput]);
        let mut stt_local = ModelEndpoint::new("stt:whisper", "stt", "whisper");
        stt_local.capabilities = Capabilities::declared(&[Capability::SpeechToText]);
        let mut stt_cloud = ModelEndpoint::new("cloud:whisper-1", "cloud", "whisper-1");
        stt_cloud.capabilities = Capabilities::declared(&[Capability::SpeechToText]);
        for m in [brain, vis, stt_local, stt_cloud] {
            r.models.insert(m.id.clone(), m);
        }
        (r, BrainProfile::new("pf", "pf", "cloud:brain"))
    }

    #[test]
    fn brain_is_preferred_when_it_has_the_capability() {
        let (r, p) = world();
        let d = resolve(
            &r,
            &p,
            &RouteRequest::for_capability(Capability::TextGeneration),
        )
        .unwrap();
        assert_eq!(d.endpoint_id, "cloud:brain");
        assert_eq!(d.reason, "brain");
    }

    #[test]
    fn vision_goes_to_the_capable_model_and_explains() {
        let (r, p) = world();
        let d = resolve(
            &r,
            &p,
            &RouteRequest::for_capability(Capability::VisionInput),
        )
        .unwrap();
        assert_eq!(d.endpoint_id, "cloud:vision");
        assert!(
            d.considered
                .iter()
                .any(|c| c.endpoint_id == "cloud:brain" && c.rejected.is_some())
        );
    }

    #[test]
    fn privacy_forces_local_stt_and_never_falls_back_to_cloud() {
        let (r, mut p) = world();
        p.privacy = PrivacyPolicy {
            transcription_local_only: true,
            ..PrivacyPolicy::default()
        };
        let mut req = RouteRequest::for_capability(Capability::SpeechToText);
        req.data = vec![DataClass::Audio];
        let ch = chain(&r, &p, &req).unwrap();
        assert_eq!(ch.len(), 1, "a nuvem nunca entra na cadeia");
        assert_eq!(ch[0].endpoint_id, "stt:whisper");
        // sem rota local: erro de privacidade, não fallback silencioso
        let (mut r2, p2) = (r.clone(), p.clone());
        r2.providers.get_mut("stt").unwrap().enabled = false;
        let e = chain(&r2, &p2, &req).unwrap_err();
        assert_eq!(e.code, ErrorCode::PrivacyPolicyBlocked, "{e}");
    }

    #[test]
    fn probed_evidence_wins_and_unverified_can_be_refused() {
        let (mut r, p) = world();
        r.models
            .get_mut("cloud:vision")
            .unwrap()
            .capabilities
            .apply_probe(&[Capability::VisionInput], &[]);
        let mut req = RouteRequest::for_capability(Capability::VisionInput);
        req.require_verified = true;
        assert_eq!(resolve(&r, &p, &req).unwrap().endpoint_id, "cloud:vision");
        let mut req = RouteRequest::for_capability(Capability::SpeechToText);
        req.require_verified = true;
        assert_eq!(
            resolve(&r, &p, &req).unwrap_err().code,
            ErrorCode::NoCapableModel
        );
    }

    #[test]
    fn ai_off_and_override_and_exclude_and_degraded_order() {
        let (mut r, mut p) = world();
        p.capability_overrides.insert(
            Capability::VisionInput,
            CapabilityPolicy::Model("cloud:vision".into()),
        );
        let mut req = RouteRequest::for_capability(Capability::VisionInput);
        assert_eq!(resolve(&r, &p, &req).unwrap().reason, "capability override");
        req.exclude = vec!["cloud:vision".into()];
        assert_eq!(
            resolve(&r, &p, &req).unwrap_err().code,
            ErrorCode::NoCapableModel
        );
        // degradado perde para saudável
        let mut two = world().0;
        let mut b2 = two.models["cloud:brain"].clone();
        b2.id = "cloud:brain2".into();
        two.models.insert(b2.id.clone(), b2);
        two.models.get_mut("cloud:brain").unwrap().health = Health::Degraded;
        let mut prof = BrainProfile::new("x", "x", "cloud:brain");
        prof.fallbacks
            .insert(Capability::ToolCalling, vec!["cloud:brain2".into()]);
        let ch = chain(
            &two,
            &prof,
            &RouteRequest::for_capability(Capability::ToolCalling),
        )
        .unwrap();
        assert_eq!(ch[0].endpoint_id, "cloud:brain2");
        // sem fallback configurado, o "auto" NÃO entra na cadeia quando o Brain serve
        let bare = BrainProfile::new("x", "x", "cloud:brain");
        let ch = chain(
            &two,
            &bare,
            &RouteRequest::for_capability(Capability::ToolCalling),
        )
        .unwrap();
        assert_eq!(
            ch.iter()
                .map(|d| d.endpoint_id.as_str())
                .collect::<Vec<_>>(),
            ["cloud:brain"]
        );
        r.ai_enabled = false;
        assert_eq!(
            resolve(&r, &p, &req).unwrap_err().code,
            ErrorCode::NotConfigured
        );
    }

    #[test]
    fn budget_blocks_route_with_known_estimate() {
        let (r, mut p) = world();
        p.budgets.max_cost_per_task_micros = Some(10);
        let mut req = RouteRequest::for_capability(Capability::TextGeneration);
        req.est_cost_micros = Some(50);
        assert_eq!(
            resolve(&r, &p, &req).unwrap_err().code,
            ErrorCode::BudgetExceeded
        );
    }
}
