//! Trait canônico de provider + fábrica. Cada adapter traduz do/para o formato nativo e normaliza
//! texto, tool calls, uso, motivo de parada e erros. Nada proprietário vaza.

use crate::cancel::CancelToken;
use crate::error::{ErrorCode, ProviderError};
use crate::registry::{ProviderConfig, ProviderKind};
use crate::stt::{SttRequest, Transcript};
use crate::types::{ChatEvent, ChatRequest, ChatResponse, ChatStream, FinishReason, ToolCallOut};
use async_trait::async_trait;
use capia_secrets::{CredentialRef, SecretStore, SecretString, register_global};
use futures_util::StreamExt;
use std::sync::Arc;

pub mod anthropic;
pub mod google;
pub mod openai;
mod openai_responses;
pub mod replay;
pub mod whisper;

/// Alça de credencial: o valor só é lido do cofre no instante de montar a requisição.
#[derive(Clone)]
pub struct SecretHandle {
    store: Option<Arc<dyn SecretStore>>,
    cred: Option<CredentialRef>,
}

impl core::fmt::Debug for SecretHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SecretHandle")
            .field("configured", &self.cred.is_some())
            .finish()
    }
}

impl SecretHandle {
    pub fn none() -> Self {
        Self {
            store: None,
            cred: None,
        }
    }

    pub fn new(store: Arc<dyn SecretStore>, cred: CredentialRef) -> Self {
        Self {
            store: Some(store),
            cred: Some(cred),
        }
    }

    pub fn is_configured(&self) -> bool {
        self.cred.is_some()
    }

    /// Lê o segredo do cofre e o registra para redação.
    pub fn resolve(&self) -> Result<Option<SecretString>, ProviderError> {
        match (&self.store, &self.cred) {
            (Some(s), Some(c)) => match s.get(c) {
                Ok(v) => {
                    register_global(v.expose());
                    Ok(Some(v))
                }
                Err(capia_secrets::StoreError::NotFound) => Err(ProviderError::new(
                    ErrorCode::NotConfigured,
                    "credential not configured for this provider",
                )),
                Err(e) => Err(ProviderError::new(ErrorCode::NotConfigured, e.to_string())),
            },
            _ => Ok(None),
        }
    }

    /// Como `resolve`, mas exige credencial.
    pub fn require(&self) -> Result<SecretString, ProviderError> {
        self.resolve()?.ok_or_else(|| {
            ProviderError::new(
                ErrorCode::NotConfigured,
                "this provider requires a credential",
            )
        })
    }
}

#[derive(Clone, Debug)]
pub struct CallCtx {
    pub cancel: CancelToken,
}

impl CallCtx {
    pub fn new(cancel: CancelToken) -> Self {
        Self { cancel }
    }
}

impl Default for CallCtx {
    fn default() -> Self {
        Self::new(CancelToken::new())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RemoteModelInfo {
    pub id: String,
    pub display_name: Option<String>,
    pub context_window: Option<u32>,
}

#[async_trait]
pub trait ModelProvider: Send + Sync + core::fmt::Debug {
    fn kind(&self) -> ProviderKind;

    fn id(&self) -> &str;

    async fn chat(&self, req: &ChatRequest, ctx: &CallCtx) -> Result<ChatStream, ProviderError>;

    async fn transcribe(
        &self,
        _req: &SttRequest,
        _ctx: &CallCtx,
    ) -> Result<Transcript, ProviderError> {
        Err(ProviderError::unsupported("speech_to_text"))
    }

    async fn list_models(&self, _ctx: &CallCtx) -> Result<Vec<RemoteModelInfo>, ProviderError> {
        Err(ProviderError::unsupported("list_models"))
    }
}

/// Constrói o adapter do provider configurado.
pub fn build_provider(
    cfg: &ProviderConfig,
    store: Arc<dyn SecretStore>,
) -> Result<Arc<dyn ModelProvider>, ProviderError> {
    cfg.validate()
        .map_err(|m| ProviderError::new(ErrorCode::InvalidRequest, m))?;
    let handle = match &cfg.credential_ref {
        Some(r) => SecretHandle::new(
            store,
            CredentialRef::new(r.clone())
                .map_err(|e| ProviderError::new(ErrorCode::InvalidRequest, e.to_string()))?,
        ),
        None => SecretHandle::none(),
    };
    match cfg.kind {
        ProviderKind::OpenAiCompatible | ProviderKind::LocalOpenAiCompatible => {
            Ok(Arc::new(openai::OpenAiProvider::new(cfg, handle)?))
        }
        ProviderKind::Anthropic => Ok(Arc::new(anthropic::AnthropicProvider::new(cfg, handle)?)),
        ProviderKind::Google => Ok(Arc::new(google::GoogleProvider::new(cfg, handle)?)),
        ProviderKind::WhisperLocal => Ok(Arc::new(whisper::WhisperProvider::new(cfg)?)),
        ProviderKind::Replay => Err(ProviderError::new(
            ErrorCode::NotConfigured,
            "the replay provider is built from fixtures (ReplayProvider::new)",
        )),
    }
}

/// Consome um stream e agrega a resposta (texto, tool calls, uso, motivo). Respeita o cancelamento.
pub async fn collect(
    mut stream: ChatStream,
    cancel: &CancelToken,
) -> Result<ChatResponse, ProviderError> {
    let mut out = ChatResponse::default();
    loop {
        let ev = tokio::select! {
            () = cancel.cancelled() => return Err(ProviderError::cancelled()),
            ev = stream.next() => ev,
        };
        match ev {
            None => break,
            Some(Err(e)) => return Err(e),
            Some(Ok(ChatEvent::TextDelta { text })) => out.text.push_str(&text),
            Some(Ok(ChatEvent::ToolCallDelta { .. })) => {}
            Some(Ok(ChatEvent::ToolCall {
                id,
                name,
                arguments,
            })) => out.tool_calls.push(ToolCallOut {
                id,
                name,
                arguments,
            }),
            Some(Ok(ChatEvent::Usage { usage })) => out.usage.add(&usage),
            Some(Ok(ChatEvent::Finish { reason })) => out.finish = Some(reason),
        }
    }
    if out.finish.is_none() {
        out.finish = Some(if out.tool_calls.is_empty() {
            FinishReason::Stop
        } else {
            FinishReason::ToolCalls
        });
    }
    Ok(out)
}

/// Acumulador de tool calls incrementais (índice → id/nome/args JSON parcial).
#[derive(Default, Debug)]
pub(crate) struct ToolAccumulator {
    calls: std::collections::BTreeMap<u32, (String, String, String)>,
}

impl ToolAccumulator {
    pub(crate) fn push(&mut self, index: u32, id: Option<&str>, name: Option<&str>, args: &str) {
        let e = self.calls.entry(index).or_default();
        if let Some(i) = id.filter(|s| !s.is_empty()) {
            e.0 = i.to_owned();
        }
        if let Some(n) = name.filter(|s| !s.is_empty()) {
            e.1 = n.to_owned();
        }
        e.2.push_str(args);
    }

    /// Fecha todas as chamadas (argumentos vazios ⇒ `{}`); JSON inválido ⇒ erro.
    pub(crate) fn finish(self) -> Result<Vec<ChatEvent>, ProviderError> {
        let mut out = Vec::new();
        for (idx, (id, name, args)) in self.calls {
            if name.is_empty() {
                return Err(ProviderError::new(
                    ErrorCode::ToolCallInvalid,
                    "tool call without a name in provider stream",
                ));
            }
            let arguments = if args.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&args).map_err(|e| {
                    ProviderError::new(
                        ErrorCode::ToolCallInvalid,
                        format!("tool call `{name}` has invalid JSON arguments: {e}"),
                    )
                })?
            };
            out.push(ChatEvent::ToolCall {
                id: if id.is_empty() {
                    format!("call_{idx}")
                } else {
                    id
                },
                name,
                arguments,
            });
        }
        Ok(out)
    }
}
