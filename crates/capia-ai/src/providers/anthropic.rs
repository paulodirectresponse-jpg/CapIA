//! Adapter Anthropic (Messages API). Tipos proprietários ficam aqui: o resto do app só vê o formato
//! canônico. Saída estruturada por **estratégia canônica**: uma tool sintética com o schema + tool
//! choice forçado; o `input` da tool volta como `TextDelta` (JSON) — nunca como tool call real.

use super::{CallCtx, ModelProvider, RemoteModelInfo, SecretHandle, ToolAccumulator};
use crate::error::{ErrorCode, ProviderError};
use crate::http::{HttpClient, SentResponse};
use crate::registry::{ProviderConfig, ProviderKind};
use crate::types::{
    ChatEvent, ChatRequest, ChatStream, FinishReason, Part, Role, ToolChoice, Usage,
};
use async_trait::async_trait;
use serde_json::{Value, json};

const API_VERSION: &str = "2023-06-01";
const STRUCTURED_TOOL: &str = "emit_structured_output";
const DEFAULT_MAX_TOKENS: u32 = 4096;

pub struct AnthropicProvider {
    id: String,
    http: HttpClient,
    secret: SecretHandle,
    extra: Vec<(String, String)>,
}

impl core::fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl AnthropicProvider {
    pub fn new(cfg: &ProviderConfig, secret: SecretHandle) -> Result<Self, ProviderError> {
        Ok(Self {
            id: cfg.id.clone(),
            http: HttpClient::new(cfg)?,
            secret,
            extra: cfg
                .extra_headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        })
    }

    fn headers(
        &self,
        mut rb: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
        let key = self.secret.require()?;
        let mut v = reqwest::header::HeaderValue::from_str(key.expose()).map_err(|_| {
            ProviderError::new(ErrorCode::InvalidRequest, "invalid credential characters")
        })?;
        v.set_sensitive(true);
        rb = rb
            .header("x-api-key", v)
            .header("anthropic-version", API_VERSION);
        for (k, v) in &self.extra {
            rb = rb.header(k, v);
        }
        Ok(rb)
    }

    pub(crate) fn build_body(&self, req: &ChatRequest) -> Result<Value, ProviderError> {
        if req.response_schema.is_some() && !req.tools.is_empty() {
            return Err(ProviderError::new(
                ErrorCode::UnsupportedCapability,
                "structured output together with tools is not supported by the Anthropic adapter",
            ));
        }
        let mut system = Vec::new();
        // [(role, blocks)] com fusão de turnos consecutivos do mesmo papel (a API exige alternância)
        let mut turns: Vec<(&'static str, Vec<Value>)> = Vec::new();
        let mut push = |role: &'static str, mut blocks: Vec<Value>| {
            if blocks.is_empty() {
                return;
            }
            match turns.last_mut() {
                Some((r, b)) if *r == role => b.append(&mut blocks),
                _ => turns.push((role, blocks)),
            }
        };
        for m in &req.messages {
            match m.role {
                Role::System => system.push(m.text_of()),
                Role::User => {
                    let mut blocks = Vec::new();
                    for p in &m.parts {
                        match p {
                            Part::Text { text } => {
                                blocks.push(json!({"type": "text", "text": text}))
                            }
                            Part::Image { mime, data_b64 } => blocks.push(json!({
                                "type": "image",
                                "source": {"type": "base64", "media_type": mime, "data": data_b64}
                            })),
                            Part::Audio { .. } => {
                                return Err(ProviderError::unsupported("audio input"));
                            }
                            _ => {}
                        }
                    }
                    push("user", blocks);
                }
                Role::Assistant => {
                    let mut blocks = Vec::new();
                    let t = m.text_of();
                    if !t.is_empty() {
                        blocks.push(json!({"type": "text", "text": t}));
                    }
                    for p in &m.parts {
                        if let Part::ToolCall {
                            id,
                            name,
                            arguments,
                        } = p
                        {
                            blocks.push(json!({"type": "tool_use", "id": id, "name": name, "input": arguments}));
                        }
                    }
                    push("assistant", blocks);
                }
                Role::Tool => {
                    let mut blocks = Vec::new();
                    for p in &m.parts {
                        if let Part::ToolResult {
                            call_id,
                            content,
                            is_error,
                            ..
                        } = p
                        {
                            blocks.push(json!({
                                "type": "tool_result", "tool_use_id": call_id,
                                "content": content, "is_error": is_error
                            }));
                        }
                    }
                    push("user", blocks);
                }
            }
        }
        let messages: Vec<Value> = turns
            .into_iter()
            .map(|(r, b)| json!({"role": r, "content": b}))
            .collect();
        let mut body = json!({
            "model": req.model,
            "max_tokens": req.params.max_output_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            "messages": messages,
            "stream": true,
        });
        if !system.is_empty() {
            body["system"] = json!(system.join("\n\n"));
        }
        if let Some(t) = req.params.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(t) = req.params.top_p {
            body["top_p"] = json!(t);
        }
        let mut tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.input_schema}))
            .collect();
        if let Some(rs) = &req.response_schema {
            tools.push(json!({
                "name": STRUCTURED_TOOL,
                "description": format!("Return the final answer as `{}` conforming exactly to this schema.", rs.name),
                "input_schema": rs.schema
            }));
            body["tool_choice"] = json!({"type": "tool", "name": STRUCTURED_TOOL});
        } else if !tools.is_empty() {
            body["tool_choice"] = match &req.tool_choice {
                ToolChoice::Auto => json!({"type": "auto"}),
                ToolChoice::None => json!({"type": "none"}),
                ToolChoice::Required => json!({"type": "any"}),
                ToolChoice::Named(n) => json!({"type": "tool", "name": n}),
            };
        }
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        Ok(body)
    }
}

fn stop_reason(s: &str) -> FinishReason {
    match s {
        "end_turn" | "stop_sequence" => FinishReason::Stop,
        "max_tokens" => FinishReason::Length,
        "tool_use" => FinishReason::ToolCalls,
        "refusal" => FinishReason::ContentFilter,
        _ => FinishReason::Other,
    }
}

struct St {
    sse: crate::http::SseReader,
    acc: ToolAccumulator,
    /// índice de bloco → índice de tool (blocos de texto não entram)
    tool_blocks: std::collections::BTreeMap<u32, u32>,
    structured_block: Option<u32>,
    structured_json: String,
    queue: std::collections::VecDeque<Result<ChatEvent, ProviderError>>,
    usage: Usage,
    finish: Option<FinishReason>,
    done: bool,
    ended: bool,
}

fn handle(st: &mut St, ev: &crate::http::SseEvent) -> Result<(), ProviderError> {
    let v: Value = serde_json::from_str(&ev.data).map_err(|e| {
        ProviderError::new(
            ErrorCode::InvalidProviderResponse,
            format!("malformed stream event: {e}"),
        )
    })?;
    let kind = ev
        .event
        .as_deref()
        .or_else(|| v.get("type").and_then(Value::as_str))
        .unwrap_or("");
    let g = |p: &str| v.pointer(p).and_then(Value::as_u64);
    match kind {
        "message_start" => {
            st.usage.input_tokens = g("/message/usage/input_tokens").unwrap_or(0);
            st.usage.cached_input_tokens = g("/message/usage/cache_read_input_tokens").unwrap_or(0);
            st.usage.output_tokens = g("/message/usage/output_tokens").unwrap_or(0);
        }
        "content_block_start" => {
            let idx = g("/index").unwrap_or(0) as u32;
            if v.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use") {
                let name = v
                    .pointer("/content_block/name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if name == STRUCTURED_TOOL {
                    st.structured_block = Some(idx);
                } else {
                    let ti = st.tool_blocks.len() as u32;
                    st.tool_blocks.insert(idx, ti);
                    st.acc.push(
                        ti,
                        v.pointer("/content_block/id").and_then(Value::as_str),
                        Some(name),
                        "",
                    );
                    st.queue.push_back(Ok(ChatEvent::ToolCallDelta {
                        index: ti,
                        name: Some(name.to_owned()),
                        arguments_fragment: String::new(),
                    }));
                }
            }
        }
        "content_block_delta" => {
            let idx = g("/index").unwrap_or(0) as u32;
            match v.pointer("/delta/type").and_then(Value::as_str) {
                Some("text_delta") => {
                    if let Some(t) = v.pointer("/delta/text").and_then(Value::as_str)
                        && !t.is_empty()
                    {
                        st.queue
                            .push_back(Ok(ChatEvent::TextDelta { text: t.to_owned() }));
                    }
                }
                Some("input_json_delta") => {
                    let frag = v
                        .pointer("/delta/partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if st.structured_block == Some(idx) {
                        st.structured_json.push_str(frag);
                        if !frag.is_empty() {
                            st.queue.push_back(Ok(ChatEvent::TextDelta {
                                text: frag.to_owned(),
                            }));
                        }
                    } else if let Some(ti) = st.tool_blocks.get(&idx) {
                        st.acc.push(*ti, None, None, frag);
                        st.queue.push_back(Ok(ChatEvent::ToolCallDelta {
                            index: *ti,
                            name: None,
                            arguments_fragment: frag.to_owned(),
                        }));
                    }
                }
                _ => {}
            }
        }
        "message_delta" => {
            if let Some(r) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                st.finish = Some(stop_reason(r));
            }
            if let Some(o) = g("/usage/output_tokens") {
                st.usage.output_tokens = o;
            }
        }
        "message_stop" => st.done = true,
        "error" => {
            let m = v
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("provider stream error");
            let code = match v.pointer("/error/type").and_then(Value::as_str) {
                Some("overloaded_error") => ErrorCode::ProviderUnavailable,
                Some("rate_limit_error") => ErrorCode::RateLimited,
                Some("authentication_error" | "permission_error") => ErrorCode::AuthFailed,
                Some("invalid_request_error") => ErrorCode::InvalidRequest,
                _ => ErrorCode::ProviderUnavailable,
            };
            return Err(ProviderError::new(code, m));
        }
        _ => {}
    }
    Ok(())
}

fn finish_events(st: &mut St) {
    let acc = std::mem::take(&mut st.acc);
    match acc.finish() {
        Ok(calls) => st.queue.extend(calls.into_iter().map(Ok)),
        Err(e) => st.queue.push_back(Err(e)),
    }
    st.queue.push_back(Ok(ChatEvent::Usage {
        usage: st.usage.clone(),
    }));
    let reason = match st.finish.take() {
        // a tool sintética de saída estruturada termina como "stop", não como tool call
        Some(FinishReason::ToolCalls) if st.structured_block.is_some() => FinishReason::Stop,
        Some(r) => r,
        None => FinishReason::Stop,
    };
    st.queue.push_back(Ok(ChatEvent::Finish { reason }));
}

fn sse_stream(resp: SentResponse, cancel: crate::CancelToken) -> ChatStream {
    let st = St {
        sse: resp.sse(cancel),
        acc: ToolAccumulator::default(),
        tool_blocks: Default::default(),
        structured_block: None,
        structured_json: String::new(),
        queue: Default::default(),
        usage: Usage::default(),
        finish: None,
        done: false,
        ended: false,
    };
    Box::pin(futures_util::stream::unfold(st, |mut st| async move {
        loop {
            if let Some(ev) = st.queue.pop_front() {
                return Some((ev, st));
            }
            if st.ended {
                return None;
            }
            if st.done {
                finish_events(&mut st);
                st.ended = true;
                continue;
            }
            match st.sse.next().await {
                Err(e) => {
                    st.done = true;
                    st.ended = true;
                    return Some((Err(e), st));
                }
                Ok(None) => {
                    st.done = true;
                }
                Ok(Some(ev)) => {
                    if let Err(e) = handle(&mut st, &ev) {
                        st.done = true;
                        st.ended = true;
                        return Some((Err(e), st));
                    }
                }
            }
        }
    }))
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }

    fn id(&self) -> &str {
        &self.id
    }

    async fn chat(&self, req: &ChatRequest, ctx: &CallCtx) -> Result<ChatStream, ProviderError> {
        let body = self.build_body(req)?;
        let rb = self.headers(self.http.post("messages")?)?.json(&body);
        let resp = self.http.send(rb, &ctx.cancel).await?;
        Ok(sse_stream(resp, ctx.cancel.clone()))
    }

    async fn list_models(&self, ctx: &CallCtx) -> Result<Vec<RemoteModelInfo>, ProviderError> {
        let rb = self.headers(self.http.get("models?limit=100")?)?;
        let v: Value = self
            .http
            .send(rb, &ctx.cancel)
            .await?
            .json(&ctx.cancel)
            .await?;
        let data = v.get("data").and_then(Value::as_array).ok_or_else(|| {
            ProviderError::new(
                ErrorCode::InvalidProviderResponse,
                "models response has no `data` array",
            )
        })?;
        Ok(data
            .iter()
            .filter_map(|m| {
                Some(RemoteModelInfo {
                    id: m.get("id")?.as_str()?.to_owned(),
                    display_name: m
                        .get("display_name")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    context_window: None,
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::types::{Message, ResponseSchema, ToolSpec};

    fn prov() -> AnthropicProvider {
        let mut c = ProviderConfig::new("a", ProviderKind::Anthropic, "a");
        c.base_url = Some("https://api.anthropic.com/v1".into());
        AnthropicProvider::new(&c, SecretHandle::none()).unwrap()
    }

    #[test]
    fn merges_turns_and_maps_tool_results() {
        let r = ChatRequest::new(
            "claude-x",
            vec![
                Message::system("s1"),
                Message::system("s2"),
                Message::user("a"),
                Message::user("b"),
                Message {
                    role: Role::Assistant,
                    parts: vec![
                        Part::text("vou usar"),
                        Part::ToolCall {
                            id: "t1".into(),
                            name: "f".into(),
                            arguments: json!({"x": 1}),
                        },
                    ],
                },
                Message {
                    role: Role::Tool,
                    parts: vec![Part::ToolResult {
                        call_id: "t1".into(),
                        name: "f".into(),
                        content: "r".into(),
                        is_error: false,
                    }],
                },
            ],
        );
        let b = prov().build_body(&r).unwrap();
        assert_eq!(b["system"], "s1\n\ns2");
        assert_eq!(
            b["messages"].as_array().unwrap().len(),
            3,
            "user+user fundidos"
        );
        assert_eq!(b["messages"][0]["content"].as_array().unwrap().len(), 2);
        assert_eq!(b["messages"][1]["content"][1]["type"], "tool_use");
        assert_eq!(b["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(b["max_tokens"], DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn structured_output_uses_forced_synthetic_tool_and_rejects_mix() {
        let mut r = ChatRequest::new("m", vec![Message::user("x")]);
        r.response_schema = Some(ResponseSchema {
            name: "demand".into(),
            schema: json!({"type": "object"}),
        });
        let b = prov().build_body(&r).unwrap();
        assert_eq!(b["tool_choice"]["name"], STRUCTURED_TOOL);
        assert_eq!(b["tools"][0]["input_schema"]["type"], "object");
        r.tools.push(ToolSpec {
            name: "t".into(),
            description: "d".into(),
            input_schema: json!({"type": "object"}),
        });
        assert_eq!(
            prov().build_body(&r).unwrap_err().code,
            ErrorCode::UnsupportedCapability
        );
    }
}
