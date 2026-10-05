//! Formato canônico de mensagens/requisições/streaming (independente de provider). Adapters traduzem
//! de/para APIs nativas; nenhum tipo proprietário vaza para a camada superior.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    /// Quadro/imagem já reduzido(a) (nunca vídeo bruto). Bytes em base64.
    Image {
        mime: String,
        data_b64: String,
    },
    Audio {
        mime: String,
        data_b64: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        call_id: String,
        name: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

impl Part {
    pub fn text(t: impl Into<String>) -> Self {
        Self::Text { text: t.into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl Message {
    pub fn system(t: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            parts: vec![Part::text(t)],
        }
    }
    pub fn user(t: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            parts: vec![Part::text(t)],
        }
    }
    pub fn assistant(t: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            parts: vec![Part::text(t)],
        }
    }
    pub fn text_of(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema do input.
    pub input_schema: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "mode", content = "name", rename_all = "snake_case")]
pub enum ToolChoice {
    #[default]
    Auto,
    None,
    Required,
    Named(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct GenParams {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_output_tokens: Option<u32>,
    pub seed: Option<u64>,
    pub reasoning_effort: Option<String>,
}

/// Saída estruturada: nome + JSON Schema (validado de volta pelo chamador).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponseSchema {
    pub name: String,
    pub schema: Value,
}

/// Metadados de rastreio (nunca enviados ao provider além do necessário).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CallMeta {
    pub task_id: Option<String>,
    pub role: Option<String>,
    pub purpose: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    /// id do modelo no provider.
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    #[serde(default)]
    pub tool_choice: ToolChoice,
    #[serde(default)]
    pub response_schema: Option<ResponseSchema>,
    #[serde(default)]
    pub params: GenParams,
    #[serde(default)]
    pub meta: CallMeta,
}

impl ChatRequest {
    pub fn new(model: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            model: model.into(),
            messages,
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            response_schema: None,
            params: GenParams::default(),
            meta: CallMeta::default(),
        }
    }

    pub fn needs_vision(&self) -> bool {
        self.messages
            .iter()
            .any(|m| m.parts.iter().any(|p| matches!(p, Part::Image { .. })))
    }

    pub fn needs_audio(&self) -> bool {
        self.messages
            .iter()
            .any(|m| m.parts.iter().any(|p| matches!(p, Part::Audio { .. })))
    }

    /// Digest determinístico (SHA-256 hex) do que *determina a resposta*: modelo, mensagens, tools,
    /// schema e parâmetros. `meta` fica de fora (não muda a resposta). Base do cache e do Replay.
    pub fn digest(&self) -> String {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::json!({
            "model": self.model,
            "messages": self.messages,
            "tools": self.tools,
            "tool_choice": self.tool_choice,
            "response_schema": self.response_schema,
            "params": self.params,
        });
        let mut h = Sha256::new();
        h.update(canonical_json(&canonical).as_bytes());
        hex(&h.finalize())
    }
}

/// JSON com chaves ordenadas (serde_json sem `preserve_order` já ordena `Map` como BTreeMap).
pub fn canonical_json(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

pub fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    /// `true` quando o número vem de um provider sintético (Replay), não de medição real.
    #[serde(default)]
    pub synthetic: bool,
}

impl Usage {
    pub fn add(&mut self, o: &Usage) {
        self.input_tokens += o.input_tokens;
        self.output_tokens += o.output_tokens;
        self.cached_input_tokens += o.cached_input_tokens;
        self.synthetic |= o.synthetic;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Other,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ChatEvent {
    TextDelta {
        text: String,
    },
    /// Fragmento incremental de uma chamada de tool (progresso na UI).
    ToolCallDelta {
        index: u32,
        name: Option<String>,
        arguments_fragment: String,
    },
    /// Chamada de tool completa (argumentos já parseados).
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    Usage {
        usage: Usage,
    },
    Finish {
        reason: FinishReason,
    },
}

pub type ChatStream =
    Pin<Box<dyn futures_util::Stream<Item = Result<ChatEvent, crate::ProviderError>> + Send>>;

/// Resposta agregada de um stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ChatResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCallOut>,
    pub usage: Usage,
    pub finish: Option<FinishReason>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCallOut {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn digest_ignores_meta_but_not_content() {
        let mut a = ChatRequest::new("m", vec![Message::user("oi")]);
        let d0 = a.digest();
        a.meta.task_id = Some("t1".into());
        assert_eq!(a.digest(), d0);
        a.messages.push(Message::user("tchau"));
        assert_ne!(a.digest(), d0);
        let mut b = ChatRequest::new("m", vec![Message::user("oi")]);
        b.params.temperature = Some(0.0);
        assert_ne!(b.digest(), d0);
    }

    #[test]
    fn serde_roundtrip() {
        let mut r = ChatRequest::new("m", vec![Message::system("s"), Message::user("u")]);
        r.tools.push(ToolSpec {
            name: "t".into(),
            description: "d".into(),
            input_schema: serde_json::json!({"type":"object"}),
        });
        r.tool_choice = ToolChoice::Named("t".into());
        let j = serde_json::to_string(&r).unwrap();
        let back: ChatRequest = serde_json::from_str(&j).unwrap();
        assert_eq!(back, r);
    }
}
