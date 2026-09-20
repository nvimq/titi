//! OpenAI-family stream decoders: Chat Completions chunks and Responses
//! lifecycle events → [`StreamEvent`] sequences.

use serde_json::Value;
use smol_str::SmolStr;

use crate::compat::StreamDecodePolicy;
use crate::sse::MarkerStripper;
use crate::stop::map_stop_reason;
use crate::stream::{BlockId, StreamEvent, ToolCallRef};
use crate::transport::ApiKind;

/// Mutable decode state for one OpenAI-family stream.
pub struct OpenAiStreamState {
    started: bool,
    text_opened: bool,
    thinking_opened: bool,
    tools_opened: Vec<bool>,
    tools: Vec<OpenToolCall>,
    seen_tools: bool,
    /// Which OpenAI endpoint family this stream belongs to (stop-reason
    /// tables differ between Completions and Responses).
    family: ApiKind,
    /// DeepSeek-class leak stripping (set by policy in transports).
    pub marker_strip: Option<MarkerStripper>,
}

impl Default for OpenAiStreamState {
    fn default() -> Self {
        Self::new(ApiKind::OpenAiCompletions)
    }
}

impl OpenAiStreamState {
    /// Construct state for the given OpenAI endpoint family.
    pub fn new(family: ApiKind) -> Self {
        Self {
            started: false,
            text_opened: false,
            thinking_opened: false,
            tools_opened: Vec::new(),
            tools: Vec::new(),
            seen_tools: false,
            family,
            marker_strip: None,
        }
    }
}

#[derive(Default)]
struct OpenToolCall {
    block_id: String,
    call_id: String,
    name: String,
}

impl OpenToolCall {
    fn new(index: usize) -> Self {
        Self {
            block_id: format!("tool_{index}"),
            ..Self::default()
        }
    }
}

fn text_id() -> BlockId {
    BlockId("text".into())
}

fn thinking_id() -> BlockId {
    BlockId("thinking".into())
}

/// Decode one Chat Completions SSE chunk into 0..N events.
pub fn decode_completions_chunk(
    payload: &Value,
    state: &mut OpenAiStreamState,
    policy: &StreamDecodePolicy,
) -> Vec<StreamEvent> {
    let mut events: Vec<StreamEvent> = Vec::new();

    let Some(choices) = payload.get("choices").and_then(Value::as_array) else {
        // Usage-only trailing chunk (stream_options.include_usage): skip.
        return events;
    };
    let Some(choice) = choices.first() else {
        return events;
    };

    if !state.started {
        state.started = true;
        events.push(StreamEvent::Start);
    }

    let Some(delta) = choice.get("delta") else {
        return events;
    };

    // Reasoning deltas (`reasoning` / `reasoning_content`).
    let reasoning = delta
        .get("reasoning")
        .or_else(|| delta.get("reasoning_content"))
        .and_then(Value::as_str);
    if let Some(text) = reasoning {
        if !state.thinking_opened {
            state.thinking_opened = true;
            events.push(StreamEvent::ThinkingStart { id: thinking_id() });
        }
        events.push(StreamEvent::ThinkingDelta {
            id: thinking_id(),
            text: text.into(),
        });
    }

    // Visible content.
    let content = match delta.get("content") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(parts)) if policy.content_is_parts_array => {
            let mut out = String::new();
            for p in parts {
                if let Some(t) = p.get("text").and_then(Value::as_str) {
                    out.push_str(t);
                }
            }
            Some(out)
        }
        _ => None,
    };
    if let Some(text) = content {
        let stripped = match &mut state.marker_strip {
            Some(s) => s.feed(&text),
            None => text,
        };
        if !stripped.is_empty() {
            if !state.text_opened {
                state.text_opened = true;
                events.push(StreamEvent::TextStart { id: text_id() });
            }
            events.push(StreamEvent::TextDelta {
                id: text_id(),
                text: stripped.into(),
            });
        }
    }

    // Streaming tool calls: indexed partial arguments.
    if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
        for tc in tool_calls {
            let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            while state.tools.len() <= index {
                let i = state.tools.len();
                state.tools.push(OpenToolCall::new(i));
                state.tools_opened.push(false);
            }
            if let Some(id) = tc.get("id").and_then(Value::as_str) {
                state.tools[index].call_id = id.to_owned();
            }
            if let Some(name) = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
            {
                state.tools[index].name = name.to_owned();
            }
            let started_now = !state.tools_opened[index] && !state.tools[index].name.is_empty();
            if started_now {
                state.tools_opened[index] = true;
                state.seen_tools = true;
                let slot = &state.tools[index];
                events.push(StreamEvent::ToolcallStart {
                    id: BlockId(slot.block_id.clone().into()),
                    call: ToolCallRef {
                        call_id: SmolStr::from(slot.call_id.clone()),
                        name: SmolStr::from(slot.name.clone()),
                    },
                });
            }
            if let Some(args) = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
            {
                // A fragment, not the accumulated buffer: consumers concatenate
                // `ToolcallDelta.json` across chunks (see the Anthropic decoder,
                // which emits `partial_json` the same way). Emitting the whole
                // buffer here would make every consumer double-count.
                events.push(StreamEvent::ToolcallDelta {
                    id: BlockId(state.tools[index].block_id.clone().into()),
                    json: args.into(),
                });
            }
        }
    }

    // Terminal finish_reason.
    if let Some(wire) = choice.get("finish_reason").and_then(Value::as_str) {
        events.extend(close_all(state, wire));
    }
    events
}

/// Emit closing events for open blocks and the terminal event, raising a
/// bare `stop` to `ToolUse` when structural tool blocks were seen.
fn close_all(state: &mut OpenAiStreamState, wire_reason: &str) -> Vec<StreamEvent> {
    let mut events = Vec::new();
    if state.thinking_opened {
        state.thinking_opened = false;
        events.push(StreamEvent::ThinkingEnd { id: thinking_id() });
    }
    // Flush any tail held by the marker stripper.
    if let Some(s) = &mut state.marker_strip {
        let tail = s.flush();
        if !tail.is_empty() {
            if !state.text_opened {
                state.text_opened = true;
                events.push(StreamEvent::TextStart { id: text_id() });
            }
            events.push(StreamEvent::TextDelta {
                id: text_id(),
                text: tail.into(),
            });
        }
    }
    if state.text_opened {
        state.text_opened = false;
        events.push(StreamEvent::TextEnd { id: text_id() });
    }
    for (i, opened) in state.tools_opened.iter_mut().enumerate() {
        if *opened {
            *opened = false;
            // No closing arguments delta: the fragments already carried
            // them, and re-sending the whole JSON would duplicate them.
            events.push(StreamEvent::ToolcallEnd {
                id: BlockId(state.tools[i].block_id.clone().into()),
            });
        }
    }
    match map_stop_reason(state.family, wire_reason) {
        crate::stop::StopMapping::Stop(reason) => {
            events.push(StreamEvent::Done {
                reason: crate::stop::promote_stop_for_tools(state.seen_tools, reason),
            });
        }
        crate::stop::StopMapping::Error(reason) => {
            events.push(StreamEvent::Error {
                reason,
                message: format!("stop reason {wire_reason:?} maps to error").into(),
            });
        }
    }
    events
}

/// Decode one Responses-API SSE event (name comes from `event:` line).
pub fn decode_responses_event(
    event: &str,
    payload: &Value,
    state: &mut OpenAiStreamState,
    _policy: &StreamDecodePolicy,
) -> Vec<StreamEvent> {
    let typ = payload.get("type").and_then(Value::as_str).unwrap_or(event);
    let mut events = Vec::new();
    match typ {
        "response.created" => {
            if !state.started {
                state.started = true;
                events.push(StreamEvent::Start);
            }
        }
        "response.output_text.delta" => {
            if !state.started {
                state.started = true;
                events.push(StreamEvent::Start);
            }
            if let Some(text) = payload.get("delta").and_then(Value::as_str) {
                if !state.text_opened {
                    state.text_opened = true;
                    events.push(StreamEvent::TextStart { id: text_id() });
                }
                events.push(StreamEvent::TextDelta {
                    id: text_id(),
                    text: text.into(),
                });
            }
        }
        "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
            if !state.started {
                state.started = true;
                events.push(StreamEvent::Start);
            }
            if let Some(text) = payload.get("delta").and_then(Value::as_str) {
                if !state.thinking_opened {
                    state.thinking_opened = true;
                    events.push(StreamEvent::ThinkingStart { id: thinking_id() });
                }
                events.push(StreamEvent::ThinkingDelta {
                    id: thinking_id(),
                    text: text.into(),
                });
            }
        }
        "response.function_call_arguments.delta" => {
            if !state.started {
                state.started = true;
                events.push(StreamEvent::Start);
            }
            let index = payload
                .get("output_index")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            while state.tools.len() <= index {
                let i = state.tools.len();
                state.tools.push(OpenToolCall::new(i));
                state.tools_opened.push(false);
            }
            if !state.tools_opened[index] {
                let name = payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let call_id = payload
                    .get("item_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if name.is_empty() {
                    return events;
                }
                state.tools_opened[index] = true;
                state.seen_tools = true;
                state.tools[index].name = name.to_owned();
                state.tools[index].call_id = call_id.to_owned();
                events.push(StreamEvent::ToolcallStart {
                    id: BlockId(state.tools[index].block_id.clone().into()),
                    call: ToolCallRef {
                        call_id: SmolStr::from(state.tools[index].call_id.clone()),
                        name: SmolStr::from(name),
                    },
                });
            }
            if let Some(args) = payload.get("delta").and_then(Value::as_str) {
                events.push(StreamEvent::ToolcallDelta {
                    id: BlockId(state.tools[index].block_id.clone().into()),
                    json: args.into(),
                });
            }
        }
        "response.completed" | "response.failed" | "response.incomplete" => {
            let wire = match typ {
                "response.completed" => "completed",
                "response.incomplete" => "incomplete",
                _ => "failed",
            };
            events.extend(close_all(state, wire));
        }
        _ => {}
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::StopReason;
    use serde_json::json;

    fn state() -> OpenAiStreamState {
        OpenAiStreamState::default()
    }
    fn responses_state() -> OpenAiStreamState {
        OpenAiStreamState::new(ApiKind::OpenAiResponses)
    }

    #[test]
    fn completions_text_flow() {
        let mut s = state();
        let policy = StreamDecodePolicy::default();
        let mut ev = decode_completions_chunk(
            &json!({"choices":[{"delta":{"role":"assistant"}}]}),
            &mut s,
            &policy,
        );
        assert_eq!(ev.remove(0), StreamEvent::Start);
        let ev = decode_completions_chunk(
            &json!({"choices":[{"delta":{"content":"Hel"}}]}),
            &mut s,
            &policy,
        );
        assert_eq!(
            ev,
            vec![
                StreamEvent::TextStart { id: text_id() },
                StreamEvent::TextDelta {
                    id: text_id(),
                    text: "Hel".into()
                }
            ]
        );
        let ev = decode_completions_chunk(
            &json!({"choices":[{"delta":{"content":"lo"},"finish_reason":null}]}),
            &mut s,
            &policy,
        );
        assert_eq!(ev.len(), 1);
        let ev = decode_completions_chunk(
            &json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
            &mut s,
            &policy,
        );
        assert_eq!(
            ev,
            vec![
                StreamEvent::TextEnd { id: text_id() },
                StreamEvent::Done {
                    reason: StopReason::Stop
                }
            ]
        );
    }

    #[test]
    fn completions_tool_call_with_partial_args() {
        let mut s = state();
        let policy = StreamDecodePolicy::default();
        let _ = decode_completions_chunk(
            &json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":""}}]}}]}),
            &mut s,
            &policy,
        );
        let ev = decode_completions_chunk(
            &json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]}}]}),
            &mut s,
            &policy,
        );
        // A fragment, never the accumulated buffer: consumers concatenate.
        assert_eq!(
            ev,
            vec![StreamEvent::ToolcallDelta {
                id: BlockId("tool_0".into()),
                json: "{\"path\":".into()
            }]
        );
        let ev = decode_completions_chunk(
            &json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"a\"}"}}]}}]}),
            &mut s,
            &policy,
        );
        assert_eq!(
            ev,
            vec![StreamEvent::ToolcallDelta {
                id: BlockId("tool_0".into()),
                json: "\"a\"}".into()
            }]
        );

        let ev = decode_completions_chunk(
            &json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            &mut s,
            &policy,
        );
        // Closing emits the end only — re-sending the whole JSON would make a
        // concatenating consumer duplicate the arguments.
        assert_eq!(
            ev,
            vec![
                StreamEvent::ToolcallEnd {
                    id: BlockId("tool_0".into())
                },
                StreamEvent::Done {
                    reason: StopReason::ToolUse
                }
            ]
        );
    }

    #[test]
    fn completions_tool_arguments_concatenate_to_valid_json() {
        let mut s = state();
        let policy = StreamDecodePolicy::default();
        let chunks = [
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read","arguments":""}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"a.rs\"}"}}]}}]}),
        ];
        let mut args = String::new();
        for chunk in &chunks {
            for event in decode_completions_chunk(chunk, &mut s, &policy) {
                if let StreamEvent::ToolcallDelta { json, .. } = event {
                    args.push_str(&json);
                }
            }
        }
        assert_eq!(args, r#"{"path":"a.rs"}"#);
        let parsed: Value = serde_json::from_str(&args).expect("concatenated arguments parse");
        assert_eq!(parsed["path"], "a.rs");
    }

    #[test]
    fn completions_stop_promoted_to_tooluse() {
        let mut s = state();
        let policy = StreamDecodePolicy::default();
        let _ = decode_completions_chunk(
            &json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"f","arguments":"{}"}}]}}]}),
            &mut s,
            &policy,
        );
        let ev = decode_completions_chunk(
            &json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
            &mut s,
            &policy,
        );
        assert!(matches!(
            ev.last(),
            Some(StreamEvent::Done {
                reason: StopReason::ToolUse
            })
        ));
    }

    #[test]
    fn malformed_tool_args_repaired_not_panicked() {
        let mut s = state();
        let policy = StreamDecodePolicy::default();
        let _ = decode_completions_chunk(
            &json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"f","arguments":""}}]}}]}),
            &mut s,
            &policy,
        );
        // Garbage arguments must not panic; finalize repairs to {}.
        let ev = decode_completions_chunk(
            &json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"garbage"}}]}}]}),
            &mut s,
            &policy,
        );
        assert!(!ev.is_empty());
        let ev = decode_completions_chunk(
            &json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            &mut s,
            &policy,
        );
        let last = ev.last().expect("terminal");
        assert!(matches!(last, StreamEvent::Done { .. }));
    }

    #[test]
    fn usage_only_chunk_skipped() {
        let mut s = state();
        let policy = StreamDecodePolicy::default();
        let ev = decode_completions_chunk(&json!({"usage":{"total_tokens":10}}), &mut s, &policy);
        assert!(ev.is_empty());
    }

    #[test]
    fn reasoning_and_content_both_streamed() {
        let mut s = state();
        let policy = StreamDecodePolicy::default();
        let ev = decode_completions_chunk(
            &json!({"choices":[{"delta":{"reasoning_content":"hmm"}}]}),
            &mut s,
            &policy,
        );
        assert!(
            ev.iter()
                .any(|e| matches!(e, StreamEvent::ThinkingDelta { .. }))
        );
        let ev = decode_completions_chunk(
            &json!({"choices":[{"delta":{"content":"answer"}}]}),
            &mut s,
            &policy,
        );
        assert!(
            ev.iter()
                .any(|e| matches!(e, StreamEvent::TextDelta { .. }))
        );
    }

    #[test]
    fn responses_lifecycle() {
        let mut s = responses_state();
        let policy = StreamDecodePolicy::default();
        let ev = decode_responses_event(
            "response.created",
            &json!({"type":"response.created"}),
            &mut s,
            &policy,
        );
        assert_eq!(ev, vec![StreamEvent::Start]);
        let ev = decode_responses_event(
            "response.output_text.delta",
            &json!({"type":"response.output_text.delta","delta":"hi"}),
            &mut s,
            &policy,
        );
        assert_eq!(
            ev,
            vec![
                StreamEvent::TextStart { id: text_id() },
                StreamEvent::TextDelta {
                    id: text_id(),
                    text: "hi".into()
                }
            ]
        );
        let ev = decode_responses_event(
            "response.completed",
            &json!({"type":"response.completed"}),
            &mut s,
            &policy,
        );
        assert_eq!(
            ev,
            vec![
                StreamEvent::TextEnd { id: text_id() },
                StreamEvent::Done {
                    reason: StopReason::Stop
                }
            ]
        );
    }

    #[test]
    fn responses_failed_maps_to_error() {
        let mut s = responses_state();
        let policy = StreamDecodePolicy::default();
        let ev = decode_responses_event(
            "response.failed",
            &json!({"type":"response.failed"}),
            &mut s,
            &policy,
        );
        assert!(matches!(ev.last(), Some(StreamEvent::Error { .. })));
    }

    #[test]
    fn unknown_responses_event_ignored() {
        let mut s = responses_state();
        let policy = StreamDecodePolicy::default();
        let ev = decode_responses_event(
            "response.heartbeat",
            &json!({"type":"response.heartbeat"}),
            &mut s,
            &policy,
        );
        assert!(ev.is_empty());
    }
}
