//! OpenAI nativo: `POST /v1/responses` (texto, streaming, tools, saída estruturada, imagem,
//! uso e cancelamento). Usado quando a base URL é `api.openai.com`; os demais endpoints
//! OpenAI-compatíveis continuam em `chat/completions` (o `kind` segue `open_ai_compatible`).

use super::ToolAccumulator;
use super::openai::OpenAiProvider;
use crate::error::{ErrorCode, ProviderError};
use crate::http::SentResponse;
use crate::types::{
    ChatEvent, ChatRequest, ChatStream, FinishReason, Part, Role, ToolChoice, Usage,
};
use serde_json::{Value, json};

fn invalid(msg: impl AsRef<str>) -> ProviderError {
    ProviderError::new(ErrorCode::InvalidProviderResponse, msg)
}

pub(crate) fn usage_of(u: &Value) -> Usage {
    let g = |p: &str| u.pointer(p).and_then(Value::as_u64).unwrap_or(0);
    Usage {
        input_tokens: g("/input_tokens"),
        output_tokens: g("/output_tokens"),
        cached_input_tokens: g("/input_tokens_details/cached_tokens"),
        synthetic: false,
    }
}

impl OpenAiProvider {
    pub(crate) fn build_responses_body(&self, req: &ChatRequest) -> Result<Value, ProviderError> {
        let mut input: Vec<Value> = Vec::new();
        for m in &req.messages {
            match m.role {
                Role::System => input.push(json!({"role": "developer", "content": m.text_of()})),
                Role::User => {
                    let mut parts = Vec::new();
                    for p in &m.parts {
                        match p {
                            Part::Text { text } => {
                                parts.push(json!({"type": "input_text", "text": text}));
                            }
                            Part::Image { mime, data_b64 } => parts.push(json!({
                                "type": "input_image",
                                "image_url": format!("data:{mime};base64,{data_b64}")
                            })),
                            Part::Audio { .. } => {
                                return Err(ProviderError::unsupported(
                                    "audio input over the Responses API (use speech-to-text)",
                                ));
                            }
                            _ => {}
                        }
                    }
                    input.push(json!({"role": "user", "content": parts}));
                }
                Role::Assistant => {
                    let text = m.text_of();
                    if !text.is_empty() {
                        input.push(json!({"role": "assistant",
                            "content": [{"type": "output_text", "text": text}]}));
                    }
                    for p in &m.parts {
                        if let Part::ToolCall {
                            id,
                            name,
                            arguments,
                        } = p
                        {
                            input.push(json!({"type": "function_call", "call_id": id,
                                "name": name, "arguments": arguments.to_string()}));
                        }
                    }
                }
                Role::Tool => {
                    for p in &m.parts {
                        if let Part::ToolResult {
                            call_id, content, ..
                        } = p
                        {
                            input.push(json!({"type": "function_call_output",
                                "call_id": call_id, "output": content}));
                        }
                    }
                }
            }
        }
        let mut body = json!({"model": req.model, "input": input, "stream": true, "store": false});
        if !req.tools.is_empty() {
            body["tools"] = json!(
                req.tools
                    .iter()
                    .map(|t| json!({"type": "function", "name": t.name,
                        "description": t.description, "parameters": t.input_schema}))
                    .collect::<Vec<_>>()
            );
            body["tool_choice"] = match &req.tool_choice {
                ToolChoice::Auto => json!("auto"),
                ToolChoice::None => json!("none"),
                ToolChoice::Required => json!("required"),
                ToolChoice::Named(n) => json!({"type": "function", "name": n}),
            };
        }
        if let Some(rs) = &req.response_schema {
            body["text"] = json!({"format": {"type": "json_schema", "name": rs.name,
                "schema": rs.schema, "strict": true}});
        }
        let p = &req.params;
        if let Some(t) = p.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(t) = p.top_p {
            body["top_p"] = json!(t);
        }
        if let Some(n) = p.max_output_tokens {
            body["max_output_tokens"] = json!(n);
        }
        if let Some(r) = &p.reasoning_effort {
            body["reasoning"] = json!({"effort": r});
        }
        Ok(body)
    }
}

fn finish_of(resp: &Value, has_calls: bool) -> FinishReason {
    if resp.get("status").and_then(Value::as_str) == Some("incomplete") {
        return match resp
            .pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
        {
            Some("content_filter") => FinishReason::ContentFilter,
            _ => FinishReason::Length,
        };
    }
    if has_calls {
        FinishReason::ToolCalls
    } else {
        FinishReason::Stop
    }
}

/// Resposta completa (não-stream, ou o objeto de `response.completed`).
pub(crate) fn events_from_response(v: &Value) -> Result<Vec<ChatEvent>, ProviderError> {
    if let Some(e) = v.get("error").filter(|e| e.is_object()) {
        return Err(ProviderError::new(
            ErrorCode::ProviderUnavailable,
            e.get("message")
                .and_then(Value::as_str)
                .unwrap_or("provider error"),
        ));
    }
    let items = v
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("response has no `output` array"))?;
    let mut out = Vec::new();
    let mut acc = ToolAccumulator::default();
    let mut n = 0u32;
    for it in items {
        match it.get("type").and_then(Value::as_str) {
            Some("message") => {
                for c in it
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(t) = c.get("text").and_then(Value::as_str)
                        && !t.is_empty()
                    {
                        out.push(ChatEvent::TextDelta { text: t.to_owned() });
                    }
                }
            }
            Some("function_call") => {
                acc.push(
                    n,
                    it.get("call_id").and_then(Value::as_str),
                    it.get("name").and_then(Value::as_str),
                    it.get("arguments").and_then(Value::as_str).unwrap_or(""),
                );
                n += 1;
            }
            _ => {}
        }
    }
    out.extend(acc.finish()?);
    if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
        out.push(ChatEvent::Usage { usage: usage_of(u) });
    }
    out.push(ChatEvent::Finish {
        reason: finish_of(v, n > 0),
    });
    Ok(out)
}

struct St {
    sse: crate::http::SseReader,
    acc: ToolAccumulator,
    /// `output_index` → índice denso do acumulador
    idx: std::collections::BTreeMap<u64, u32>,
    queue: std::collections::VecDeque<Result<ChatEvent, ProviderError>>,
    done: bool,
}

pub(crate) fn sse_stream(resp: SentResponse, cancel: crate::CancelToken) -> ChatStream {
    let st = St {
        sse: resp.sse(cancel),
        acc: ToolAccumulator::default(),
        idx: Default::default(),
        queue: Default::default(),
        done: false,
    };
    Box::pin(futures_util::stream::unfold(st, |mut st| async move {
        loop {
            if let Some(ev) = st.queue.pop_front() {
                return Some((ev, st));
            }
            if st.done {
                return None;
            }
            match st.sse.next().await {
                Err(e) => {
                    st.done = true;
                    return Some((Err(e), st));
                }
                Ok(None) => {
                    st.done = true;
                    st.queue.push_back(Err(ProviderError::new(
                        ErrorCode::ProviderUnavailable,
                        "the stream ended before the response completed",
                    )));
                }
                Ok(Some(ev)) => {
                    if ev.data.trim() == "[DONE]" {
                        st.done = true;
                        continue;
                    }
                    let v: Value = match serde_json::from_str(&ev.data) {
                        Ok(v) => v,
                        Err(e) => {
                            st.done = true;
                            return Some((
                                Err(invalid(format!("malformed stream event: {e}"))),
                                st,
                            ));
                        }
                    };
                    handle(&mut st, &v);
                }
            }
        }
    }))
}

fn handle(st: &mut St, v: &Value) {
    let oi = v.get("output_index").and_then(Value::as_u64).unwrap_or(0);
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "response.output_text.delta" => {
            if let Some(t) = v.get("delta").and_then(Value::as_str)
                && !t.is_empty()
            {
                st.queue
                    .push_back(Ok(ChatEvent::TextDelta { text: t.to_owned() }));
            }
        }
        "response.output_item.added" => {
            let it = v.get("item").unwrap_or(&Value::Null);
            if it.get("type").and_then(Value::as_str) == Some("function_call") {
                let n = u32::try_from(st.idx.len()).unwrap_or(u32::MAX);
                let n = *st.idx.entry(oi).or_insert(n);
                let name = it.get("name").and_then(Value::as_str);
                st.acc
                    .push(n, it.get("call_id").and_then(Value::as_str), name, "");
                st.queue.push_back(Ok(ChatEvent::ToolCallDelta {
                    index: n,
                    name: name.map(str::to_owned),
                    arguments_fragment: String::new(),
                }));
            }
        }
        "response.function_call_arguments.delta" => {
            let frag = v.get("delta").and_then(Value::as_str).unwrap_or("");
            let n = u32::try_from(st.idx.len()).unwrap_or(u32::MAX);
            let n = *st.idx.entry(oi).or_insert(n);
            st.acc.push(n, None, None, frag);
            st.queue.push_back(Ok(ChatEvent::ToolCallDelta {
                index: n,
                name: None,
                arguments_fragment: frag.to_owned(),
            }));
        }
        "response.completed" | "response.incomplete" => {
            st.done = true;
            let r = v.get("response").unwrap_or(&Value::Null);
            let acc = std::mem::take(&mut st.acc);
            let calls = acc.finish();
            let has = matches!(&calls, Ok(c) if !c.is_empty());
            match calls {
                Ok(c) => st.queue.extend(c.into_iter().map(Ok)),
                Err(e) => st.queue.push_back(Err(e)),
            }
            if let Some(u) = r.get("usage").filter(|u| u.is_object()) {
                st.queue
                    .push_back(Ok(ChatEvent::Usage { usage: usage_of(u) }));
            }
            let mut r2 = r.clone();
            if v.get("type").and_then(Value::as_str) == Some("response.incomplete") {
                r2["status"] = json!("incomplete");
            }
            st.queue.push_back(Ok(ChatEvent::Finish {
                reason: finish_of(&r2, has),
            }));
        }
        "response.failed" => {
            st.done = true;
            let m = v
                .pointer("/response/error/message")
                .and_then(Value::as_str)
                .unwrap_or("the provider reported a failed response");
            st.queue
                .push_back(Err(ProviderError::new(ErrorCode::ProviderUnavailable, m)));
        }
        "error" => {
            st.done = true;
            let m = v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("provider stream error");
            st.queue
                .push_back(Err(ProviderError::new(ErrorCode::ProviderUnavailable, m)));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::providers::SecretHandle;
    use crate::registry::{ProviderConfig, ProviderKind};
    use crate::types::{Message, ToolSpec};

    fn prov() -> OpenAiProvider {
        let mut c = ProviderConfig::new("o", ProviderKind::OpenAiCompatible, "o");
        c.base_url = Some("https://api.openai.com/v1".into());
        OpenAiProvider::new(&c, SecretHandle::none()).unwrap()
    }

    #[test]
    fn body_maps_to_the_responses_shape() {
        let mut r = ChatRequest::new(
            "gpt-x",
            vec![
                Message::system("sys"),
                Message {
                    role: Role::User,
                    parts: vec![
                        Part::text("olha"),
                        Part::Image {
                            mime: "image/png".into(),
                            data_b64: "AAAA".into(),
                        },
                    ],
                },
                Message {
                    role: Role::Assistant,
                    parts: vec![Part::ToolCall {
                        id: "c1".into(),
                        name: "t".into(),
                        arguments: json!({"a": 1}),
                    }],
                },
                Message {
                    role: Role::Tool,
                    parts: vec![Part::ToolResult {
                        call_id: "c1".into(),
                        name: "t".into(),
                        content: "ok".into(),
                        is_error: false,
                    }],
                },
            ],
        );
        r.tools = vec![ToolSpec {
            name: "t".into(),
            description: "d".into(),
            input_schema: json!({"type": "object"}),
        }];
        r.params.max_output_tokens = Some(50);
        let b = prov().build_responses_body(&r).unwrap();
        assert_eq!(b["input"][0]["role"], "developer");
        assert_eq!(b["input"][1]["content"][0]["type"], "input_text");
        assert_eq!(b["input"][1]["content"][1]["type"], "input_image");
        assert_eq!(b["input"][2]["type"], "function_call");
        assert_eq!(b["input"][3]["type"], "function_call_output");
        assert_eq!(b["tools"][0]["name"], "t");
        assert_eq!(b["max_output_tokens"], 50);
        assert_eq!(b["store"], false);
        assert!(b.get("messages").is_none());
    }

    #[test]
    fn complete_response_yields_text_calls_usage_and_finish() {
        let v = json!({"status": "completed", "output": [
            {"type": "message", "content": [{"type": "output_text", "text": "oi"}]},
            {"type": "function_call", "call_id": "c9", "name": "t", "arguments": "{\"x\":2}"}],
            "usage": {"input_tokens": 7, "output_tokens": 3,
                      "input_tokens_details": {"cached_tokens": 2}}});
        let ev = events_from_response(&v).unwrap();
        assert!(matches!(&ev[0], ChatEvent::TextDelta { text } if text == "oi"));
        assert!(
            matches!(&ev[1], ChatEvent::ToolCall { id, name, .. } if id == "c9" && name == "t")
        );
        assert!(
            matches!(&ev[2], ChatEvent::Usage { usage } if usage.input_tokens == 7 && usage.cached_input_tokens == 2)
        );
        assert!(matches!(
            ev.last().unwrap(),
            ChatEvent::Finish {
                reason: FinishReason::ToolCalls
            }
        ));
    }

    #[test]
    fn failed_response_is_an_error() {
        let v = json!({"error": {"message": "bad key"}, "output": []});
        assert!(events_from_response(&v).is_err());
    }
}
