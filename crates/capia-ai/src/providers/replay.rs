//! Provider **Replay**: determinístico, sem rede, sem custo (CI). Casa por *digest* do pedido,
//! simula streaming, tool calls, saída estruturada, uso sintético (explicitamente marcado) e erros.
//! Também há modo *script* (respostas em ordem) para cenários de retry/fallback. Fixtures são
//! versionadas, redigidas e **nunca** carregam segredo.

use super::{CallCtx, ModelProvider, RemoteModelInfo};
use crate::error::{ErrorCode, ProviderError};
use crate::registry::ProviderKind;
use crate::stt::{SttRequest, Transcript};
use crate::types::{ChatEvent, ChatRequest, ChatStream, FinishReason, Usage};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

pub const FIXTURE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplayResponse {
    Chat {
        events: Vec<ChatEvent>,
        #[serde(default)]
        chunk_delay_ms: u64,
    },
    /// Falha **depois** de emitir `events` (stream interrompido) ou imediatamente (vazio).
    Error {
        code: ErrorCode,
        message: String,
        #[serde(default)]
        status: Option<u16>,
        #[serde(default)]
        retry_after_ms: Option<u64>,
        #[serde(default)]
        after_events: Vec<ChatEvent>,
    },
    Transcript {
        transcript: Transcript,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayEntry {
    /// `ChatRequest::digest()` (ou chave de STT).
    pub digest: String,
    #[serde(default)]
    pub note: String,
    pub response: ReplayResponse,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayFixture {
    pub version: u32,
    pub entries: Vec<ReplayEntry>,
}

impl ReplayFixture {
    pub fn new(entries: Vec<ReplayEntry>) -> Self {
        Self {
            version: FIXTURE_VERSION,
            entries,
        }
    }

    pub fn from_json(s: &str) -> Result<Self, String> {
        let f: Self =
            serde_json::from_str(s).map_err(|e| format!("invalid replay fixture: {e}"))?;
        if f.version != FIXTURE_VERSION {
            return Err(format!(
                "unsupported replay fixture version {} (expected {FIXTURE_VERSION})",
                f.version
            ));
        }
        Ok(f)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

/// Chave de STT: o áudio entra só pelo SHA-256 (a fixture nunca carrega mídia).
pub fn stt_digest(req: &SttRequest) -> String {
    use sha2::{Digest, Sha256};
    let audio = crate::types::hex(&Sha256::digest(&req.audio));
    let canon = serde_json::json!({
        "model": req.model, "audio_sha256": audio, "language": req.language, "words": req.word_timestamps,
    });
    crate::types::hex(&Sha256::digest(canon.to_string().as_bytes()))
}

/// Respondedor programático (testes): vê o pedido e o nº da chamada (0, 1, …).
pub type Responder = Box<dyn Fn(&ChatRequest, u32) -> ReplayResponse + Send + Sync>;

enum Mode {
    ByDigest(BTreeMap<String, ReplayResponse>),
    Script(Mutex<VecDeque<ReplayResponse>>),
    Responder(Responder),
}

pub struct ReplayProvider {
    id: String,
    mode: Mode,
    models: Vec<RemoteModelInfo>,
    /// Digests recebidos (testes verificam o que foi pedido).
    pub seen: Mutex<Vec<String>>,
    /// Pedidos completos recebidos (só para testes de introspecção; o Replay não faz rede).
    pub requests: Mutex<Vec<ChatRequest>>,
    /// Quantas chamadas chegaram (inclui as que falharam).
    pub calls: Mutex<u32>,
}

impl core::fmt::Debug for ReplayProvider {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReplayProvider")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl ReplayProvider {
    pub fn new(id: impl Into<String>, fixture: ReplayFixture) -> Self {
        let map = fixture
            .entries
            .into_iter()
            .map(|e| (e.digest, e.response))
            .collect();
        Self::build(id, Mode::ByDigest(map))
    }

    /// Respostas em ordem, ignorando o digest (retry/fallback/erros).
    pub fn scripted(id: impl Into<String>, script: Vec<ReplayResponse>) -> Self {
        Self::build(id, Mode::Script(Mutex::new(script.into())))
    }

    /// Respostas calculadas a partir do pedido (ex.: devolver o `plan_token` que veio da tool).
    pub fn responder(id: impl Into<String>, f: Responder) -> Self {
        Self::build(id, Mode::Responder(f))
    }

    fn build(id: impl Into<String>, mode: Mode) -> Self {
        Self {
            id: id.into(),
            mode,
            models: vec![RemoteModelInfo {
                id: "replay-brain".into(),
                display_name: Some("Replay".into()),
                context_window: Some(128_000),
            }],
            seen: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
            calls: Mutex::new(0),
        }
    }

    pub fn with_models(mut self, models: Vec<RemoteModelInfo>) -> Self {
        self.models = models;
        self
    }

    pub fn call_count(&self) -> u32 {
        self.calls.lock().map_or(0, |c| *c)
    }

    fn next(&self, digest: &str) -> Result<ReplayResponse, ProviderError> {
        if let Ok(mut c) = self.calls.lock() {
            *c += 1;
        }
        if let Ok(mut s) = self.seen.lock() {
            s.push(digest.to_owned());
        }
        match &self.mode {
            Mode::ByDigest(m) => m.get(digest).cloned().ok_or_else(|| {
                ProviderError::new(
                    ErrorCode::InvalidRequest,
                    format!("no replay fixture for request digest {digest}"),
                )
            }),
            Mode::Responder(_) => Err(ProviderError::new(
                ErrorCode::InvalidRequest,
                "responder replay is only used for chat",
            )),
            Mode::Script(q) => q
                .lock()
                .map_err(|_| ProviderError::new(ErrorCode::ProviderUnavailable, "replay poisoned"))?
                .pop_front()
                .ok_or_else(|| {
                    ProviderError::new(ErrorCode::InvalidRequest, "replay script exhausted")
                }),
        }
    }
}

fn estimate_usage(req: &ChatRequest, events: &[ChatEvent]) -> Usage {
    let chars_in: usize = req.messages.iter().map(|m| m.text_of().len()).sum();
    let chars_out: usize = events
        .iter()
        .map(|e| match e {
            ChatEvent::TextDelta { text } => text.len(),
            ChatEvent::ToolCall { arguments, .. } => arguments.to_string().len(),
            _ => 0,
        })
        .sum();
    Usage {
        input_tokens: (chars_in / 4).max(1) as u64,
        output_tokens: (chars_out / 4).max(1) as u64,
        cached_input_tokens: 0,
        synthetic: true,
    }
}

struct Playback {
    events: VecDeque<ChatEvent>,
    delay: Duration,
    then_error: Option<ProviderError>,
    cancel: crate::CancelToken,
    first: bool,
}

fn stream_of(
    events: Vec<ChatEvent>,
    delay_ms: u64,
    then_error: Option<ProviderError>,
    cancel: crate::CancelToken,
) -> ChatStream {
    let st = Playback {
        events: events.into(),
        delay: Duration::from_millis(delay_ms),
        then_error,
        cancel,
        first: true,
    };
    Box::pin(futures_util::stream::unfold(st, |mut st| async move {
        if !st.delay.is_zero() && !st.first {
            tokio::select! {
                () = st.cancel.cancelled() => return Some((Err(ProviderError::cancelled()), st)),
                () = tokio::time::sleep(st.delay) => {}
            }
        }
        st.first = false;
        if st.cancel.is_cancelled() {
            st.events.clear();
            st.then_error = None;
            return Some((Err(ProviderError::cancelled()), st));
        }
        if let Some(e) = st.events.pop_front() {
            return Some((Ok(e), st));
        }
        st.then_error.take().map(|e| (Err(e), st))
    }))
}

#[async_trait]
impl ModelProvider for ReplayProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Replay
    }

    fn id(&self) -> &str {
        &self.id
    }

    async fn chat(&self, req: &ChatRequest, ctx: &CallCtx) -> Result<ChatStream, ProviderError> {
        if let Ok(mut r) = self.requests.lock() {
            r.push(req.clone());
        }
        let resp = if let Mode::Responder(f) = &self.mode {
            let n = {
                let mut c = self.calls.lock().map_err(|_| {
                    ProviderError::new(ErrorCode::ProviderUnavailable, "replay poisoned")
                })?;
                let n = *c;
                *c += 1;
                n
            };
            f(req, n)
        } else {
            self.next(&req.digest())?
        };
        match resp {
            ReplayResponse::Chat {
                mut events,
                chunk_delay_ms,
            } => {
                if !events.iter().any(|e| matches!(e, ChatEvent::Usage { .. })) {
                    let u = estimate_usage(req, &events);
                    events.push(ChatEvent::Usage { usage: u });
                }
                if !events.iter().any(|e| matches!(e, ChatEvent::Finish { .. })) {
                    let calls = events
                        .iter()
                        .any(|e| matches!(e, ChatEvent::ToolCall { .. }));
                    events.push(ChatEvent::Finish {
                        reason: if calls {
                            FinishReason::ToolCalls
                        } else {
                            FinishReason::Stop
                        },
                    });
                }
                Ok(stream_of(events, chunk_delay_ms, None, ctx.cancel.clone()))
            }
            ReplayResponse::Error {
                code,
                message,
                status,
                retry_after_ms,
                after_events,
            } => {
                let mut e = ProviderError::new(code, message);
                e.status = status;
                e.retry_after_ms = retry_after_ms;
                if after_events.is_empty() {
                    Err(e)
                } else {
                    Ok(stream_of(after_events, 0, Some(e), ctx.cancel.clone()))
                }
            }
            ReplayResponse::Transcript { .. } => Err(ProviderError::new(
                ErrorCode::InvalidRequest,
                "fixture holds a transcript, not a chat response",
            )),
        }
    }

    async fn transcribe(
        &self,
        req: &SttRequest,
        _ctx: &CallCtx,
    ) -> Result<Transcript, ProviderError> {
        match self.next(&stt_digest(req))? {
            ReplayResponse::Transcript { transcript } => Ok(transcript),
            ReplayResponse::Error {
                code,
                message,
                status,
                retry_after_ms,
                ..
            } => {
                let mut e = ProviderError::new(code, message);
                e.status = status;
                e.retry_after_ms = retry_after_ms;
                Err(e)
            }
            ReplayResponse::Chat { .. } => Err(ProviderError::new(
                ErrorCode::InvalidRequest,
                "fixture holds a chat response, not a transcript",
            )),
        }
    }

    async fn list_models(&self, _ctx: &CallCtx) -> Result<Vec<RemoteModelInfo>, ProviderError> {
        Ok(self.models.clone())
    }
}

/// Grava as respostas de outro provider como fixture (para gerar Replay a partir de uma sessão real).
pub struct Recorder {
    entries: Mutex<Vec<ReplayEntry>>,
}

impl core::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Recorder").finish_non_exhaustive()
    }
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    pub fn record_chat(&self, req: &ChatRequest, resp: &crate::types::ChatResponse, note: &str) {
        let mut events = Vec::new();
        if !resp.text.is_empty() {
            events.push(ChatEvent::TextDelta {
                text: capia_secrets::redact(&resp.text),
            });
        }
        for c in &resp.tool_calls {
            events.push(ChatEvent::ToolCall {
                id: c.id.clone(),
                name: c.name.clone(),
                arguments: c.arguments.clone(),
            });
        }
        // o uso real vira sintético (a fixture não afirma medição)
        let mut u = resp.usage.clone();
        u.synthetic = true;
        events.push(ChatEvent::Usage { usage: u });
        if let Some(r) = resp.finish {
            events.push(ChatEvent::Finish { reason: r });
        }
        if let Ok(mut e) = self.entries.lock() {
            e.push(ReplayEntry {
                digest: req.digest(),
                note: capia_secrets::redact(note),
                response: ReplayResponse::Chat {
                    events,
                    chunk_delay_ms: 0,
                },
            });
        }
    }

    pub fn into_fixture(self) -> ReplayFixture {
        ReplayFixture::new(self.entries.into_inner().unwrap_or_default())
    }
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::providers::collect;
    use crate::types::Message;

    fn req(t: &str) -> ChatRequest {
        ChatRequest::new("replay-brain", vec![Message::user(t)])
    }

    fn fixture_for(r: &ChatRequest, text: &str) -> ReplayFixture {
        ReplayFixture::new(vec![ReplayEntry {
            digest: r.digest(),
            note: String::new(),
            response: ReplayResponse::Chat {
                events: vec![
                    ChatEvent::TextDelta {
                        text: text[..2].into(),
                    },
                    ChatEvent::TextDelta {
                        text: text[2..].into(),
                    },
                ],
                chunk_delay_ms: 0,
            },
        }])
    }

    #[tokio::test]
    async fn matches_by_digest_streams_and_marks_usage_synthetic() {
        let r = req("olá");
        let p = ReplayProvider::new("rp", fixture_for(&r, "resposta"));
        let ctx = CallCtx::default();
        let out = collect(p.chat(&r, &ctx).await.unwrap(), &ctx.cancel)
            .await
            .unwrap();
        assert_eq!(out.text, "resposta");
        assert!(out.usage.synthetic && out.usage.output_tokens >= 1);
        // pedido diferente ⇒ erro explícito com o digest
        let e = p.chat(&req("outro"), &ctx).await.err().unwrap();
        assert_eq!(e.code, ErrorCode::InvalidRequest);
        assert!(e.message.contains("digest"));
    }

    #[tokio::test]
    async fn script_mode_errors_then_success_and_midstream_failure() {
        let p = ReplayProvider::scripted(
            "rp",
            vec![
                ReplayResponse::Error {
                    code: ErrorCode::RateLimited,
                    message: "slow down".into(),
                    status: Some(429),
                    retry_after_ms: Some(10),
                    after_events: vec![],
                },
                ReplayResponse::Error {
                    code: ErrorCode::ProviderUnavailable,
                    message: "cut".into(),
                    status: None,
                    retry_after_ms: None,
                    after_events: vec![ChatEvent::TextDelta { text: "par".into() }],
                },
                ReplayResponse::Chat {
                    events: vec![ChatEvent::TextDelta { text: "ok".into() }],
                    chunk_delay_ms: 0,
                },
            ],
        );
        let ctx = CallCtx::default();
        let r = req("x");
        let e1 = p.chat(&r, &ctx).await.err().unwrap();
        assert_eq!(
            (e1.code, e1.retry_after_ms),
            (ErrorCode::RateLimited, Some(10))
        );
        let e2 = collect(p.chat(&r, &ctx).await.unwrap(), &ctx.cancel)
            .await
            .unwrap_err();
        assert_eq!(e2.code, ErrorCode::ProviderUnavailable);
        let ok = collect(p.chat(&r, &ctx).await.unwrap(), &ctx.cancel)
            .await
            .unwrap();
        assert_eq!(ok.text, "ok");
        assert_eq!(p.call_count(), 3);
    }

    #[tokio::test]
    async fn cancel_aborts_playback() {
        let r = req("x");
        let p = ReplayProvider::new(
            "rp",
            ReplayFixture::new(vec![ReplayEntry {
                digest: r.digest(),
                note: String::new(),
                response: ReplayResponse::Chat {
                    events: (0..50)
                        .map(|i| ChatEvent::TextDelta {
                            text: format!("{i} "),
                        })
                        .collect(),
                    chunk_delay_ms: 20,
                },
            }]),
        );
        let ctx = CallCtx::default();
        let cancel = ctx.cancel.clone();
        let stream = p.chat(&r, &ctx).await.unwrap();
        let h = tokio::spawn({
            let c = cancel.clone();
            async move { collect(stream, &c).await }
        });
        tokio::time::sleep(Duration::from_millis(60)).await;
        cancel.cancel();
        let e = h.await.unwrap().unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
    }

    #[test]
    fn fixture_roundtrip_and_version_check() {
        let r = req("a");
        let f = fixture_for(&r, "abcd");
        let back = ReplayFixture::from_json(&f.to_json()).unwrap();
        assert_eq!(back, f);
        assert!(ReplayFixture::from_json(r#"{"version": 99, "entries": []}"#).is_err());
    }

    #[test]
    fn recorder_never_keeps_secrets_in_note_or_text() {
        let rec = Recorder::new();
        let r = req("x");
        let resp = crate::types::ChatResponse {
            text: "key=sk-abcdefghijklmnopqrstuvwxyz".into(),
            ..Default::default()
        };
        rec.record_chat(&r, &resp, "Authorization: Bearer tok_abcdefghijk");
        let json = rec.into_fixture().to_json();
        assert!(!json.contains("abcdefghijklmnop"), "{json}");
        assert!(!json.contains("tok_abcdefghijk"), "{json}");
    }
}
