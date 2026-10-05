//! Adapter Google Gemini (`streamGenerateContent`, SSE). A chave vai no header `x-goog-api-key`
//! (nunca na query: URLs vazam em logs/erros). Schema JSON → subconjunto OpenAPI do Gemini.

use super::{CallCtx, ModelProvider, RemoteModelInfo, SecretHandle};
use crate::error::{ErrorCode, ProviderError};
use crate::http::{HttpClient, SentResponse};
use crate::registry::{ProviderConfig, ProviderKind};
use crate::types::{
    ChatEvent, ChatRequest, ChatStream, FinishReason, Part, Role, ToolChoice, Usage,
};
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct GoogleProvider {
    id: String,
    http: HttpClient,
    secret: SecretHandle,
    extra: Vec<(String, String)>,
}

impl core::fmt::Debug for GoogleProvider {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GoogleProvider")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl GoogleProvider {
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
        rb = rb.header("x-goog-api-key", v);
        for (k, v) in &self.extra {
            rb = rb.header(k, v);
        }
        Ok(rb)
    }

    pub(crate) fn build_body(&self, req: &ChatRequest) -> Result<Value, ProviderError> {
        let mut system = Vec::new();
        let mut contents: Vec<Value> = Vec::new();
        for m in &req.messages {
            match m.role {
                Role::System => system.push(m.text_of()),
                Role::User => {
                    let mut parts = Vec::new();
                    for p in &m.parts {
                        match p {
                            Part::Text { text } => parts.push(json!({"text": text})),
                            Part::Image { mime, data_b64 } | Part::Audio { mime, data_b64 } => {
                                parts.push(
                                    json!({"inlineData": {"mimeType": mime, "data": data_b64}}),
                                );
                            }
                            _ => {}
                        }
                    }
                    contents.push(json!({"role": "user", "parts": parts}));
                }
                Role::Assistant => {
                    let mut parts = Vec::new();
                    let t = m.text_of();
                    if !t.is_empty() {
                        parts.push(json!({"text": t}));
                    }
                    for p in &m.parts {
                        if let Part::ToolCall {
                            name, arguments, ..
                        } = p
                        {
                            parts.push(json!({"functionCall": {"name": name, "args": arguments}}));
                        }
                    }
                    contents.push(json!({"role": "model", "parts": parts}));
                }
                Role::Tool => {
                    let mut parts = Vec::new();
                    for p in &m.parts {
                        if let Part::ToolResult {
                            name,
                            content,
                            is_error,
                            ..
                        } = p
                        {
                            let resp = serde_json::from_str::<Value>(content)
                                .ok()
                                .filter(Value::is_object)
                                .unwrap_or_else(|| json!({"result": content}));
                            let resp = if *is_error {
                                json!({"error": resp})
                            } else {
                                resp
                            };
                            parts.push(
                                json!({"functionResponse": {"name": name, "response": resp}}),
                            );
                        }
                    }
                    contents.push(json!({"role": "user", "parts": parts}));
                }
            }
        }
        let mut body = json!({"contents": contents});
        if !system.is_empty() {
            body["systemInstruction"] = json!({"parts": [{"text": system.join("\n\n")}]});
        }
        if !req.tools.is_empty() {
            let decls: Vec<Value> = req
                .tools
                .iter()
                .map(|t| json!({"name": t.name, "description": t.description, "parameters": gemini_schema(&t.input_schema)}))
                .collect();
            body["tools"] = json!([{"functionDeclarations": decls}]);
            body["toolConfig"] = json!({"functionCallingConfig": match &req.tool_choice {
                ToolChoice::Auto => json!({"mode": "AUTO"}),
                ToolChoice::None => json!({"mode": "NONE"}),
                ToolChoice::Required => json!({"mode": "ANY"}),
                ToolChoice::Named(n) => json!({"mode": "ANY", "allowedFunctionNames": [n]}),
            }});
        }
        let mut genc = json!({});
        let p = &req.params;
        if let Some(t) = p.temperature {
            genc["temperature"] = json!(t);
        }
        if let Some(t) = p.top_p {
            genc["topP"] = json!(t);
        }
        if let Some(n) = p.max_output_tokens {
            genc["maxOutputTokens"] = json!(n);
        }
        if let Some(s) = p.seed {
            genc["seed"] = json!(s);
        }
        if let Some(rs) = &req.response_schema {
            if !req.tools.is_empty() {
                return Err(ProviderError::new(
                    ErrorCode::UnsupportedCapability,
                    "structured output together with tools is not supported by the Google adapter",
                ));
            }
            genc["responseMimeType"] = json!("application/json");
            genc["responseSchema"] = gemini_schema(&rs.schema);
        }
        if genc.as_object().is_some_and(|o| !o.is_empty()) {
            body["generationConfig"] = genc;
        }
        Ok(body)
    }
}

/// JSON Schema → subconjunto aceito pelo Gemini (tipos em maiúsculas; sem `$schema`,
/// `additionalProperties`, `$defs`, `title`).
pub(crate) fn gemini_schema(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut out = serde_json::Map::new();
            for (k, val) in m {
                match k.as_str() {
                    "$schema"
                    | "additionalProperties"
                    | "$defs"
                    | "definitions"
                    | "title"
                    | "default"
                    | "examples" => {}
                    "type" => {
                        let t = match val {
                            Value::String(s) => json!(s.to_ascii_uppercase()),
                            // ["string","null"] → STRING + nullable
                            Value::Array(a) => {
                                let non_null: Vec<&Value> =
                                    a.iter().filter(|x| x.as_str() != Some("null")).collect();
                                if a.iter().any(|x| x.as_str() == Some("null")) {
                                    out.insert("nullable".into(), json!(true));
                                }
                                non_null
                                    .first()
                                    .and_then(|x| x.as_str())
                                    .map_or(json!("STRING"), |s| json!(s.to_ascii_uppercase()))
                            }
                            other => other.clone(),
                        };
                        out.insert(k.clone(), t);
                    }
                    _ => {
                        out.insert(k.clone(), gemini_schema(val));
                    }
                }
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(gemini_schema).collect()),
        other => other.clone(),
    }
}

fn finish_of(s: &str, had_call: bool) -> FinishReason {
    if had_call {
        return FinishReason::ToolCalls;
    }
    match s {
        "STOP" => FinishReason::Stop,
        "MAX_TOKENS" => FinishReason::Length,
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" => {
            FinishReason::ContentFilter
        }
        _ => FinishReason::Other,
    }
}

struct St {
    sse: crate::http::SseReader,
    queue: std::collections::VecDeque<Result<ChatEvent, ProviderError>>,
    usage: Usage,
    finish: Option<String>,
    had_call: bool,
    calls: u32,
    done: bool,
    ended: bool,
}

fn handle(st: &mut St, data: &str) -> Result<(), ProviderError> {
    let v: Value = serde_json::from_str(data).map_err(|e| {
        ProviderError::new(
            ErrorCode::InvalidProviderResponse,
            format!("malformed stream event: {e}"),
        )
    })?;
    if let Some(err) = v.get("error") {
        let m = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("provider stream error");
        return Err(ProviderError::new(ErrorCode::ProviderUnavailable, m));
    }
    if let Some(u) = v.get("usageMetadata") {
        let g = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        st.usage = Usage {
            input_tokens: g("promptTokenCount"),
            output_tokens: g("candidatesTokenCount") + g("thoughtsTokenCount"),
            cached_input_tokens: g("cachedContentTokenCount"),
            synthetic: false,
        };
    }
    if let Some(r) = v
        .pointer("/promptFeedback/blockReason")
        .and_then(Value::as_str)
    {
        return Err(ProviderError::new(
            ErrorCode::ContentFiltered,
            format!("prompt blocked: {r}"),
        ));
    }
    let Some(cand) = v.pointer("/candidates/0") else {
        return Ok(());
    };
    if let Some(parts) = cand.pointer("/content/parts").and_then(Value::as_array) {
        for p in parts {
            if let Some(t) = p.get("text").and_then(Value::as_str)
                && !t.is_empty()
                && p.get("thought").and_then(Value::as_bool) != Some(true)
            {
                st.queue
                    .push_back(Ok(ChatEvent::TextDelta { text: t.to_owned() }));
            }
            if let Some(fc) = p.get("functionCall") {
                let name = fc.get("name").and_then(Value::as_str).unwrap_or("");
                if name.is_empty() {
                    return Err(ProviderError::new(
                        ErrorCode::ToolCallInvalid,
                        "function call without a name",
                    ));
                }
                let args = fc.get("args").cloned().unwrap_or_else(|| json!({}));
                let idx = st.calls;
                st.calls += 1;
                st.had_call = true;
                st.queue.push_back(Ok(ChatEvent::ToolCallDelta {
                    index: idx,
                    name: Some(name.to_owned()),
                    arguments_fragment: args.to_string(),
                }));
                st.queue.push_back(Ok(ChatEvent::ToolCall {
                    id: format!("call_{idx}"),
                    name: name.to_owned(),
                    arguments: args,
                }));
            }
        }
    }
    if let Some(r) = cand.get("finishReason").and_then(Value::as_str) {
        st.finish = Some(r.to_owned());
    }
    Ok(())
}

fn sse_stream(resp: SentResponse, cancel: crate::CancelToken) -> ChatStream {
    let st = St {
        sse: resp.sse(cancel),
        queue: Default::default(),
        usage: Usage::default(),
        finish: None,
        had_call: false,
        calls: 0,
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
                st.queue.push_back(Ok(ChatEvent::Usage {
                    usage: st.usage.clone(),
                }));
                let reason = finish_of(st.finish.as_deref().unwrap_or("STOP"), st.had_call);
                st.queue.push_back(Ok(ChatEvent::Finish { reason }));
                st.ended = true;
                continue;
            }
            match st.sse.next().await {
                Err(e) => {
                    st.ended = true;
                    return Some((Err(e), st));
                }
                Ok(None) => st.done = true,
                Ok(Some(ev)) => {
                    if let Err(e) = handle(&mut st, &ev.data) {
                        st.ended = true;
                        return Some((Err(e), st));
                    }
                }
            }
        }
    }))
}

#[async_trait]
impl ModelProvider for GoogleProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Google
    }

    fn id(&self) -> &str {
        &self.id
    }

    async fn chat(&self, req: &ChatRequest, ctx: &CallCtx) -> Result<ChatStream, ProviderError> {
        if req.model.is_empty()
            || !req
                .model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(ProviderError::new(
                ErrorCode::InvalidRequest,
                "invalid model id",
            ));
        }
        let body = self.build_body(req)?;
        let path = format!("models/{}:streamGenerateContent?alt=sse", req.model);
        let rb = self.headers(self.http.post(&path)?)?.json(&body);
        let resp = self.http.send(rb, &ctx.cancel).await?;
        Ok(sse_stream(resp, ctx.cancel.clone()))
    }

    async fn list_models(&self, ctx: &CallCtx) -> Result<Vec<RemoteModelInfo>, ProviderError> {
        let rb = self.headers(self.http.get("models?pageSize=200")?)?;
        let v: Value = self
            .http
            .send(rb, &ctx.cancel)
            .await?
            .json(&ctx.cancel)
            .await?;
        let data = v.get("models").and_then(Value::as_array).ok_or_else(|| {
            ProviderError::new(
                ErrorCode::InvalidProviderResponse,
                "models response has no `models` array",
            )
        })?;
        Ok(data
            .iter()
            .filter_map(|m| {
                let name = m.get("name")?.as_str()?;
                Some(RemoteModelInfo {
                    id: name.strip_prefix("models/").unwrap_or(name).to_owned(),
                    display_name: m
                        .get("displayName")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    context_window: m
                        .get("inputTokenLimit")
                        .and_then(Value::as_u64)
                        .map(|n| n as u32),
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

    fn prov() -> GoogleProvider {
        let mut c = ProviderConfig::new("g", ProviderKind::Google, "g");
        c.base_url = Some("https://generativelanguage.googleapis.com/v1beta".into());
        GoogleProvider::new(&c, SecretHandle::none()).unwrap()
    }

    #[test]
    fn schema_conversion_for_gemini() {
        let s = json!({"$schema": "x", "type": "object", "additionalProperties": false,
            "properties": {"a": {"type": ["string", "null"]}, "b": {"type": "array", "items": {"type": "integer"}}}});
        let g = gemini_schema(&s);
        assert!(g.get("$schema").is_none() && g.get("additionalProperties").is_none());
        assert_eq!(g["type"], "OBJECT");
        assert_eq!(g["properties"]["a"]["type"], "STRING");
        assert_eq!(g["properties"]["a"]["nullable"], true);
        assert_eq!(g["properties"]["b"]["items"]["type"], "INTEGER");
    }

    #[test]
    fn body_maps_roles_tools_and_structured() {
        let mut r = ChatRequest::new(
            "gemini-x",
            vec![
                Message::system("s"),
                Message::user("oi"),
                Message {
                    role: Role::Assistant,
                    parts: vec![Part::ToolCall {
                        id: "c".into(),
                        name: "f".into(),
                        arguments: json!({"a": 1}),
                    }],
                },
                Message {
                    role: Role::Tool,
                    parts: vec![Part::ToolResult {
                        call_id: "c".into(),
                        name: "f".into(),
                        content: "texto".into(),
                        is_error: false,
                    }],
                },
            ],
        );
        r.tools.push(ToolSpec {
            name: "f".into(),
            description: "d".into(),
            input_schema: json!({"type": "object"}),
        });
        let b = prov().build_body(&r).unwrap();
        assert_eq!(b["systemInstruction"]["parts"][0]["text"], "s");
        assert_eq!(b["contents"][1]["role"], "model");
        assert_eq!(b["contents"][1]["parts"][0]["functionCall"]["name"], "f");
        assert_eq!(
            b["contents"][2]["parts"][0]["functionResponse"]["response"]["result"],
            "texto"
        );
        assert_eq!(b["toolConfig"]["functionCallingConfig"]["mode"], "AUTO");
        r.tools.clear();
        r.response_schema = Some(ResponseSchema {
            name: "n".into(),
            schema: json!({"type": "object"}),
        });
        let b = prov().build_body(&r).unwrap();
        assert_eq!(
            b["generationConfig"]["responseMimeType"],
            "application/json"
        );
        assert_eq!(b["generationConfig"]["responseSchema"]["type"], "OBJECT");
    }
}
