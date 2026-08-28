//! Gemini `streamGenerateContent?alt=sse` decoder: parts-chunks →
//! [`StreamEvent`].

use serde_json::Value;
use smol_str::SmolStr;

use crate::compat::StreamDecodePolicy;
use crate::stop::map_stop_reason;
use crate::stream::{BlockId, ErrorReason, StreamEvent, ToolCallRef};
use crate::transport::ApiKind;

/// Mutable decode state for one Gemini stream.
#[derive(Default)]
pub struct GeminiStreamState {
    started: bool,
    text_open: bool,
    thinking_open: bool,
    /// Open tool calls by index (Gemini has no per-index stream; each chunk
    /// carries complete functionCall parts).
    tools_opened: Vec<bool>,
}

fn text_id() -> BlockId {
    BlockId("text".into())
}

fn thinking_id() -> BlockId {
    BlockId("thinking".into())
}

fn tool_id(i: usize) -> BlockId {
    BlockId(format!("tool_{i}").into())
}

/// Decode one Gemini SSE chunk (one `candidates[0].content.parts` payload).
pub fn decode_chunk(
    payload: &Value,
    state: &mut GeminiStreamState,
    policy: &StreamDecodePolicy,
) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    // Prompt-feedback / error payloads surface as stream errors.
    if let Some(feedback) = payload.get("promptFeedback") {
        if let Some(reason) = feedback.get("blockReason").and_then(Value::as_str) {
            return vec![StreamEvent::Error {
                reason: ErrorReason::Rejected,
                message: format!("blocked by prompt feedback: {reason}").into(),
            }];
        }
    }

    let Some(candidate) =
        payload.pointer("/candidates/0") else { return events };
    let content = candidate.get("content");
    let parts = content
        .and_then(|c| c.get("parts"))
        .and_then(Value::as_array);

    if !state.started {
        state.started = true;
        events.push(StreamEvent::Start);
    }

    if let Some(parts) = parts {
        for part in parts {
            let thought = part.get("thought").and_then(Value::as_bool).unwrap_or(false);
            // Gemini tool calls arrive as complete functionCall parts.
            if let Some(call) = part.get("functionCall") {
                let name = call.get("name").and_then(Value::as_str).unwrap_or_default();
                let index = state.tools_opened.len();
                state.tools_opened.push(true);
                events.push(StreamEvent::ToolcallStart {
                    id: tool_id(index),
                    call: ToolCallRef {
                        call_id: SmolStr::from(format!("gemini_{index}")),
                        name: name.into(),
                    },
                });
                let args = call.get("args").cloned().unwrap_or(Value::Object(Default::default()));
                events.push(StreamEvent::ToolcallDelta {
                    id: tool_id(index),
                    json: args.to_string().into(),
                });
                events.push(StreamEvent::ToolcallEnd { id: tool_id(index) });
                continue;
            }
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                if thought {
                    if !state.thinking_open {
                        state.thinking_open = true;
                        events.push(StreamEvent::ThinkingStart { id: thinking_id() });
                    }
                    events.push(StreamEvent::ThinkingDelta {
                        id: thinking_id(),
                        text: text.into(),
                    });
                } else {
                    if !state.text_open {
                        state.text_open = true;
                        events.push(StreamEvent::TextStart { id: text_id() });
                    }
                    events.push(StreamEvent::TextDelta { id: text_id(), text: text.into() });
                }
                continue;
            }
            // Unknown part shape: tolerated unless parts-array policy set.
            if policy.content_is_parts_array && part.get("text").is_none() {
                // Mistral-class guard has no meaning here; skip silently.
            }
        }
    }

    // finishReason terminates the stream (may arrive in the same chunk as
    // parts).
    if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str) {
        events.extend(close(state, reason));
    }
    events
}

fn close(state: &mut GeminiStreamState, wire: &str) -> Vec<StreamEvent> {
    let mut events = Vec::new();
    if state.thinking_open {
        state.thinking_open = false;
        events.push(StreamEvent::ThinkingEnd { id: thinking_id() });
    }
    if state.text_open {
        state.text_open = false;
        events.push(StreamEvent::TextEnd { id: text_id() });
    }
    match map_stop_reason(ApiKind::GeminiGenerateContent, wire) {
        crate::stop::StopMapping::Stop(reason) => events.push(StreamEvent::Done { reason }),
        crate::stop::StopMapping::Error(reason) => {
            events.push(StreamEvent::Error {
                reason,
                message: format!("finishReason {wire} maps to error").into(),
            });
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::StopReason;
    use serde_json::json;

    #[test]
    fn text_parts_flow() {
        let mut s = GeminiStreamState::default();
        let policy = StreamDecodePolicy::default();
        let ev = decode_chunk(
            &json!({"candidates":[{"content":{"parts":[{"text":"Hel"}]}}]}),
            &mut s,
            &policy,
        );
        assert_eq!(
            ev,
            vec![
                StreamEvent::Start,
                StreamEvent::TextStart { id: text_id() },
                StreamEvent::TextDelta { id: text_id(), text: "Hel".into() }
            ]
        );
        let ev = decode_chunk(
            &json!({"candidates":[{"content":{"parts":[{"text":"lo"}]},"finishReason":"STOP"}]}),
            &mut s,
            &policy,
        );
        assert_eq!(
            ev,
            vec![
                StreamEvent::TextDelta { id: text_id(), text: "lo".into() },
                StreamEvent::TextEnd { id: text_id() },
                StreamEvent::Done { reason: StopReason::Stop }
            ]
        );
    }

    #[test]
    fn thought_parts_are_thinking() {
        let mut s = GeminiStreamState::default();
        let policy = StreamDecodePolicy::default();
        let ev = decode_chunk(
            &json!({"candidates":[{"content":{"parts":[{"text":"ponder","thought":true}]}}]}),
            &mut s,
            &policy,
        );
        assert!(ev.iter().any(|e| matches!(e, StreamEvent::ThinkingDelta { .. })));
        assert!(!ev.iter().any(|e| matches!(e, StreamEvent::TextDelta { .. })));
    }

    #[test]
    fn function_call_parts() {
        let mut s = GeminiStreamState::default();
        let policy = StreamDecodePolicy::default();
        let ev = decode_chunk(
            &json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"search","args":{"q":"rust"}}}]}}]}),
            &mut s,
            &policy,
        );
        assert!(matches!(&ev[1], StreamEvent::ToolcallStart { call, .. } if call.name == "search"));
        assert!(matches!(&ev[2], StreamEvent::ToolcallDelta { json, .. } if json.contains("\"q\"")));
        assert!(matches!(ev[3], StreamEvent::ToolcallEnd { .. }));
    }

    #[test]
    fn stop_reason_table_gemini() {
        let mut s = GeminiStreamState::default();
        let policy = StreamDecodePolicy::default();
        let check = |wire: &str, s: &mut GeminiStreamState| {
            decode_chunk(
                &json!({"candidates":[{"finishReason":wire}]}),
                s,
                &StreamDecodePolicy::default(),
            )
            .pop()
            .expect("terminal")
        };
        assert_eq!(check("STOP", &mut s), StreamEvent::Done { reason: StopReason::Stop });
        assert_eq!(check("MAX_TOKENS", &mut s), StreamEvent::Done { reason: StopReason::Length });
        assert!(matches!(check("SAFETY", &mut s), StreamEvent::Error { .. }));
        assert!(matches!(check("MALFORMED_FUNCTION_CALL", &mut s), StreamEvent::Error { .. }));
        let _ = policy;
    }

    #[test]
    fn blocked_prompt_becomes_error() {
        let mut s = GeminiStreamState::default();
        let ev = decode_chunk(
            &json!({"promptFeedback":{"blockReason":"SAFETY"}}),
            &mut s,
            &StreamDecodePolicy::default(),
        );
        assert!(matches!(ev[0], StreamEvent::Error { reason: ErrorReason::Rejected, .. }));
    }
}
