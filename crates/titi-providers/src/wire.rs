//! Family transports: one SSE pump shared by all endpoint families, with a
//! per-family HTTP request shape. Dispatch is by [`ApiKind`], never provider
//! name.

use futures::{Stream, StreamExt};
use serde_json::Value;
use smol_str::SmolStr;
use std::pin::Pin;
use std::sync::Arc;

use crate::compat::StreamDecodePolicy;
use crate::http::{BodyChunk, HttpFetch, HttpRequest, ReqwestFetch};
use crate::sse::{SseDecoder, SseFrame};
use crate::stream::{ErrorReason, StopReason, StreamEvent};
use crate::transport::{
    ApiKind, EventStream, RequestCtx, Role, Transport, TransportError, WatchdogConfig,
    WireRequest,
};

// ---------------------------------------------------------------------------
// Wire serialization (normalized → family request)
// ---------------------------------------------------------------------------

impl Role {
    fn openai_role(&self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }

    fn anthropic_role(&self) -> Option<&'static str> {
        match self {
            Role::System => None, // system goes to the top-level field
            Role::User | Role::Tool => Some("user"),
            Role::Assistant => Some("assistant"),
        }
    }
}

fn openai_messages_wire(req: &WireRequest) -> Vec<Value> {
    let mut out = Vec::with_capacity(req.messages.len() + 1);
    if let Some(sys) = &req.system {
        out.push(serde_json::json!({"role": "system", "content": sys.as_str()}));
    }
    for m in &req.messages {
        out.push(serde_json::json!({"role": m.role.openai_role(), "content": m.content.as_str()}));
    }
    out
}

fn anthropic_messages_wire(req: &WireRequest) -> Vec<Value> {
    req.messages
        .iter()
        .filter_map(|m| {
            m.role.anthropic_role().map(|role| {
                serde_json::json!({"role": role, "content": m.content.as_str()})
            })
        })
        .collect()
}

fn gemini_contents_wire(req: &WireRequest) -> Vec<Value> {
    req.messages
        .iter()
        .filter_map(|m| match m.role {
            Role::User | Role::Tool => Some(serde_json::json!(
                {"role": "user", "parts": [{"text": m.content.as_str()}]}
            )),
            Role::Assistant => Some(serde_json::json!(
                {"role": "model", "parts": [{"text": m.content.as_str()}]}
            )),
            Role::System => None, // systemInstruction field
        })
        .collect()
}

fn openai_tools_wire(req: &WireRequest) -> Vec<Value> {
    req.tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name.as_str(),
                    "description": t.description.as_str(),
                    "parameters": t.parameters,
                }
            })
        })
        .collect()
}

fn anthropic_tools_wire(req: &WireRequest) -> Vec<Value> {
    req.tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name.as_str(),
                "description": t.description.as_str(),
                "input_schema": t.parameters,
            })
        })
        .collect()
}

fn gemini_tools_wire(req: &WireRequest) -> Vec<Value> {
    if req.tools.is_empty() {
        return Vec::new();
    }
    vec![serde_json::json!({
        "functionDeclarations": req.tools.iter().map(|t| serde_json::json!({
            "name": t.name.as_str(),
            "description": t.description.as_str(),
            "parameters": t.parameters,
        })).collect::<Vec<_>>()
    })]
}

/// Serialize a normalized request into a family-specific HTTP request.
pub fn build_http_request(
    api: ApiKind,
    base_url: &str,
    req: &WireRequest,
    api_key: Option<&str>,
) -> HttpRequest {
    let auth_headers = |headers: &mut Vec<(SmolStr, SmolStr)>, key: Option<&str>, style: &str| {
        if let Some(k) = key {
            match style {
                "anthropic" => {
                    headers.push(("x-api-key".into(), k.into()));
                    headers.push(("anthropic-version".into(), "2023-06-01".into()));
                }
                "query" => {} // Gemini key appended to URL
                _ => headers.push(("authorization".into(), format!("Bearer {k}").into())),
            }
        }
    };
    match api {
        ApiKind::OpenAiCompletions => {
            let mut headers = vec![
                ("content-type".into(), "application/json".into()),
                ("accept".into(), "text/event-stream".into()),
            ];
            auth_headers(&mut headers, api_key, "bearer");
            let body = serde_json::json!({
                "model": req.model.as_str(),
                "messages": openai_messages_wire(req),
                "stream": true,
                "tools": openai_tools_wire(req),
                "max_tokens": req.max_tokens,
                "temperature": req.temperature,
            });
            HttpRequest {
                method: "POST".into(),
                url: format!("{}/chat/completions", base_url.trim_end_matches('/')).into(),
                headers,
                body: Some(body.to_string().into_bytes()),
            }
        }
        ApiKind::OpenAiResponses => {
            let mut headers = vec![
                ("content-type".into(), "application/json".into()),
                ("accept".into(), "text/event-stream".into()),
            ];
            auth_headers(&mut headers, api_key, "bearer");
            let mut body = serde_json::json!({
                "model": req.model.as_str(),
                "input": openai_messages_wire(req),
                "stream": true,
                "tools": openai_tools_wire(req),
                "max_output_tokens": req.max_tokens,
            });
            if let Some(sys) = &req.system {
                body["instructions"] = Value::String(sys.to_string());
            }
            HttpRequest {
                method: "POST".into(),
                url: format!("{}/responses", base_url.trim_end_matches('/')).into(),
                headers,
                body: Some(body.to_string().into_bytes()),
            }
        }
        ApiKind::AnthropicMessages => {
            let mut headers = vec![
                ("content-type".into(), "application/json".into()),
                ("accept".into(), "text/event-stream".into()),
            ];
            auth_headers(&mut headers, api_key, "anthropic");
            let body = serde_json::json!({
                "model": req.model.as_str(),
                "messages": anthropic_messages_wire(req),
                "system": req.system.as_deref().unwrap_or_default(),
                "stream": true,
                "tools": anthropic_tools_wire(req),
                "max_tokens": req.max_tokens.unwrap_or(4096),
            });
            HttpRequest {
                method: "POST".into(),
                url: format!("{}/v1/messages", base_url.trim_end_matches('/')).into(),
                headers,
                body: Some(body.to_string().into_bytes()),
            }
        }
        ApiKind::GeminiGenerateContent => {
            let mut headers = vec![("content-type".into(), "application/json".into())];
            auth_headers(&mut headers, api_key, "query");
            let mut url = format!(
                "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
                base_url.trim_end_matches('/'),
                req.model.as_str()
            );
            if let Some(k) = api_key {
                url.push_str(&format!("&key={k}"));
            }
            let mut body = serde_json::json!({
                "contents": gemini_contents_wire(req),
                "tools": gemini_tools_wire(req),
            });
            if let Some(sys) = &req.system {
                body["systemInstruction"] =
                    serde_json::json!({"parts": [{"text": sys.as_str()}]});
            }
            if let Some(max) = req.max_tokens {
                body["generationConfig"] =
                    serde_json::json!({"maxOutputTokens": max});
            }
            HttpRequest {
                method: "POST".into(),
                url: url.into(),
                headers,
                body: Some(body.to_string().into_bytes()),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shared SSE pump: byte stream → SseFrames → family decoder → StreamEvents
// ---------------------------------------------------------------------------

/// Per-family mutable decode state.
pub enum FamilyDecoder {
    OpenAi(crate::openai::OpenAiStreamState),
    Anthropic(crate::anthropic::AnthropicStreamState),
    Gemini(crate::gemini::GeminiStreamState),
}

struct PumpState {
    body: Pin<Box<dyn Stream<Item = Result<BodyChunk, String>> + Send>>,
    decoder: SseDecoder,
    api: ApiKind,
    policy: StreamDecodePolicy,
    family: FamilyDecoder,
    /// Events decoded from frames already read, drained before polling more
    /// bytes (a single SSE frame can decode to several events).
    queued: std::collections::VecDeque<StreamEvent>,
    done: bool,
}

impl PumpState {
    fn decode_frame(&mut self, frame: SseFrame) {
        let SseFrame::Data { event, data } = frame else {
            return;
        };
        let payload: Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => {
                self.queued.push_back(StreamEvent::Error {
                    reason: ErrorReason::Malformed,
                    message: format!("{api}: undecodable SSE data payload", api = self.api)
                        .into(),
                });
                return;
            }
        };
        let events = match &mut self.family {
            FamilyDecoder::OpenAi(s) => match self.api {
                ApiKind::OpenAiResponses => {
                    crate::openai::decode_responses_event(&event, &payload, s, &self.policy)
                }
                _ => crate::openai::decode_completions_chunk(&payload, s, &self.policy),
            },
            FamilyDecoder::Anthropic(s) => {
                crate::anthropic::decode_event(&event, &payload, s)
            }
            FamilyDecoder::Gemini(s) => crate::gemini::decode_chunk(&payload, s, &self.policy),
        };
        self.queued.extend(events);
    }
}

/// Pump raw body bytes into normalized events.
pub fn sse_event_stream(
    body: Pin<Box<dyn Stream<Item = Result<BodyChunk, String>> + Send>>,
    api: ApiKind,
    policy: StreamDecodePolicy,
) -> impl Stream<Item = StreamEvent> + Send {
    let family = match api {
        ApiKind::AnthropicMessages => FamilyDecoder::Anthropic(Default::default()),
        ApiKind::GeminiGenerateContent => FamilyDecoder::Gemini(Default::default()),
        _ => FamilyDecoder::OpenAi(crate::openai::OpenAiStreamState::new(api)),
    };
    futures::stream::unfold(
        PumpState {
            body: Box::pin(body),
            decoder: SseDecoder::new(),
            api,
            policy,
            family,
            queued: std::collections::VecDeque::new(),
            done: false,
        },
        pump_step,
    )
}

async fn pump_step(mut state: PumpState) -> Option<(StreamEvent, PumpState)> {
    loop {
        if let Some(ev) = state.queued.pop_front() {
            if ev.is_terminal() {
                state.done = true;
            }
            return Some((ev, state));
        }
        if state.done {
            return None;
        }
        match state.body.next().await {
            Some(Ok(chunk)) => {
                for frame in state.decoder.feed(&chunk) {
                    state.decode_frame(frame);
                }
            }
            Some(Err(e)) => {
                state.done = true;
                return Some((
                    StreamEvent::Error { reason: ErrorReason::Connection, message: e.into() },
                    state,
                ));
            }
            None => {
                state.done = true;
                if let Some(frame) = state.decoder.finish() {
                    state.decode_frame(frame);
                }
                // Fall through: next loop iteration drains decoded events or
                // emits the default Done.
                if let Some(ev) = state.queued.pop_front() {
                    return Some((ev, state));
                }
                return Some((StreamEvent::Done { reason: StopReason::Stop }, state));
            }
        }
    }
}


// ---------------------------------------------------------------------------
// Transports
// ---------------------------------------------------------------------------

/// Generic family transport over an injectable [`HttpFetch`].
pub struct FamilyTransport {
    api: ApiKind,
    base_url: SmolStr,
    fetch: Arc<dyn HttpFetch>,
}

impl FamilyTransport {
    pub fn new(
        api: ApiKind,
        base_url: impl Into<SmolStr>,
        fetch: Arc<dyn HttpFetch>,
    ) -> Self {
        Self { api, base_url: base_url.into(), fetch }
    }

    pub fn with_default_fetch(
        api: ApiKind,
        base_url: impl Into<SmolStr>,
    ) -> Result<Self, TransportError> {
        Ok(Self {
            api,
            base_url: base_url.into(),
            fetch: Arc::new(ReqwestFetch::new()?),
        })
    }
}

#[async_trait::async_trait]
impl Transport for FamilyTransport {
    fn api(&self) -> ApiKind {
        self.api
    }

    fn watchdog(&self) -> WatchdogConfig {
        WatchdogConfig::default()
    }

    async fn stream(
        &self,
        req: WireRequest,
        ctx: RequestCtx,
    ) -> Result<EventStream, TransportError> {
        let http_req =
            build_http_request(self.api, &self.base_url, &req, ctx.api_key.as_deref());
        let resp = self.fetch.fetch(http_req).await?;
        if resp.status == 429 || resp.status >= 500 {
            return Err(TransportError::Retryable {
                status: Some(resp.status),
                message: format!("upstream status {}", resp.status).into(),
            });
        }
        if resp.status >= 400 {
            return Err(TransportError::Fatal {
                status: Some(resp.status),
                message: format!("upstream status {}", resp.status).into(),
            });
        }
        Ok(Box::pin(sse_event_stream(resp.body, self.api, StreamDecodePolicy::default())))
    }
}

/// OpenAI-compatible Chat Completions transport (OpenRouter, vLLM, Ollama
/// compat endpoints, gateways). Any compatible base URL works — dispatch is
/// by [`ApiKind`].
pub type OpenAiCompatTransport = FamilyTransport;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ChatMessage;

    fn req() -> WireRequest {
        let mut r = WireRequest::new("gpt-test");
        r.system = Some("be brief".into());
        r.messages = vec![
            ChatMessage { role: Role::User, content: "hi".into(), tool_calls: Vec::new() },
            ChatMessage { role: Role::Assistant, content: "hello".into(), tool_calls: Vec::new() },
        ];
        r.max_tokens = Some(128);
        r
    }

    #[test]
    fn openai_completions_wire_shape() {
        let r = req();
        let hr = build_http_request(ApiKind::OpenAiCompletions, "http://x/v1", &r, Some("sk"));
        assert!(hr.url.ends_with("/chat/completions"));
        let body: Value = serde_json::from_slice(hr.body.as_ref().expect("body")).expect("json");
        assert_eq!(body["model"], "gpt-test");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 128);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        let auth = hr.headers.iter().find(|(k, _)| k == "authorization").expect("auth");
        assert_eq!(auth.1, "Bearer sk");
    }

    #[test]
    fn openai_responses_wire_shape() {
        let r = req();
        let hr = build_http_request(ApiKind::OpenAiResponses, "http://x/v1", &r, None);
        assert!(hr.url.ends_with("/responses"));
        let body: Value = serde_json::from_slice(hr.body.as_ref().expect("body")).expect("json");
        assert_eq!(body["max_output_tokens"], 128);
        assert_eq!(body["instructions"], "be brief");
        assert!(body["input"].is_array());
    }

    #[test]
    fn anthropic_wire_shape() {
        let r = req();
        let hr = build_http_request(ApiKind::AnthropicMessages, "http://x", &r, Some("k"));
        assert!(hr.url.ends_with("/v1/messages"));
        let body: Value = serde_json::from_slice(hr.body.as_ref().expect("body")).expect("json");
        assert_eq!(body["system"], "be brief");
        assert_eq!(body["max_tokens"], 128);
        assert!(body["messages"].as_array().expect("msgs").iter().all(|m| m["role"] != "system"));
        let key = hr.headers.iter().find(|(k, _)| k == "x-api-key").expect("key");
        assert_eq!(key.1, "k");
        let ver = hr.headers.iter().find(|(k, _)| k == "anthropic-version").expect("ver");
        assert_eq!(ver.1, "2023-06-01");
    }

    #[test]
    fn gemini_wire_shape() {
        let r = req();
        let hr =
            build_http_request(ApiKind::GeminiGenerateContent, "http://x", &r, Some("gk"));
        assert!(hr.url.contains(":streamGenerateContent?alt=sse"));
        assert!(hr.url.contains("key=gk"));
        let body: Value = serde_json::from_slice(hr.body.as_ref().expect("body")).expect("json");
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "be brief");
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 128);
    }

    #[test]
    fn openrouter_style_omits_max_tokens_when_absent() {
        let mut r = WireRequest::new("m");
        r.messages = vec![ChatMessage {
            role: Role::User,
            content: "x".into(),
            tool_calls: Vec::new(),
        }];
        let hr = build_http_request(ApiKind::OpenAiCompletions, "http://x", &r, None);
        let body: Value = serde_json::from_slice(hr.body.as_ref().expect("body")).expect("json");
        assert!(body.get("max_tokens").map(Value::is_null).unwrap_or(true));
    }
}
