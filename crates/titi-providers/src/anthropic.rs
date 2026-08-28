//! Anthropic Messages stream decoder: `content_block_*` lifecycle + typed
//! deltas → [`StreamEvent`].

use serde_json::Value;

use crate::partial_json::PartialJson;
use crate::stop::map_stop_reason;
use crate::stream::{BlockId, ErrorReason, StreamEvent, ToolCallRef};
use crate::transport::ApiKind;

/// Mutable decode state for one Anthropic stream.
#[derive(Default)]
pub struct AnthropicStreamState {
    started: bool,
    /// index → open block kind
    open: Vec<BlockKind>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BlockKind {
    Text,
    Thinking,
    Tool,
}

fn malformed(message: impl Into<String>) -> StreamEvent {
    StreamEvent::Error { reason: ErrorReason::Malformed, message: message.into().into() }
}

fn block_id(kind: BlockKind, index: usize) -> BlockId {
    BlockId(match kind {
        BlockKind::Text => format!("text_{index}"),
        BlockKind::Thinking => format!("thinking_{index}"),
        BlockKind::Tool => format!("tool_{index}"),
    }
    .into())
}

/// Decode one Anthropic Messages SSE event.
pub fn decode_event(
    event: &str,
    payload: &Value,
    state: &mut AnthropicStreamState,
) -> Vec<StreamEvent> {
    let typ = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(event);
    let mut events = Vec::new();
    match typ {
        "message_start" => {
            if !state.started {
                state.started = true;
                events.push(StreamEvent::Start);
            }
        }
        "content_block_start" => {
            let index = payload.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            while state.open.len() <= index {
                state.open.push(BlockKind::Text);
            }
            let block = payload.get("content_block").cloned().unwrap_or(Value::Null);
            let kind = match block.get("type").and_then(Value::as_str) {
                Some("thinking") | Some("redacted_thinking") => BlockKind::Thinking,
                Some("tool_use") => BlockKind::Tool,
                _ => BlockKind::Text,
            };
            state.open[index] = kind;
            match kind {
                BlockKind::Text => {
                    events.push(StreamEvent::TextStart { id: block_id(kind, index) });
                }
                BlockKind::Thinking => {
                    events.push(StreamEvent::ThinkingStart { id: block_id(kind, index) });
                }
                BlockKind::Tool => {
                    let call = ToolCallRef {
                        call_id: block.get("id").and_then(Value::as_str).unwrap_or_default().into(),
                        name: block.get("name").and_then(Value::as_str).unwrap_or_default().into(),
                    };
                    events.push(StreamEvent::ToolcallStart {
                        id: block_id(kind, index),
                        call,
                    });
                }
            }
        }
        "content_block_delta" => {
            let index = payload.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let kind = state.open.get(index).copied();
            let delta = payload.get("delta").cloned().unwrap_or(Value::Null);
            match (kind, delta.get("type").and_then(Value::as_str)) {
                (Some(BlockKind::Tool), Some("input_json_delta")) => {
                    if let Some(partial) = delta.get("partial_json").and_then(Value::as_str) {
                        events.push(StreamEvent::ToolcallDelta {
                            id: block_id(BlockKind::Tool, index),
                            json: partial.into(),
                        });
                    }
                }
                (Some(BlockKind::Thinking), Some("thinking_delta")) => {
                    if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                        events.push(StreamEvent::ThinkingDelta {
                            id: block_id(BlockKind::Thinking, index),
                            text: text.into(),
                        });
                    }
                }
                (Some(BlockKind::Text), Some("text_delta")) => {
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        events.push(StreamEvent::TextDelta {
                            id: block_id(BlockKind::Text, index),
                            text: text.into(),
                        });
                    }
                }
                (Some(BlockKind::Text), Some("input_json_delta"))
                | (Some(BlockKind::Thinking), Some("input_json_delta")) => {
                    events.push(malformed(format!(
                        "input_json_delta on {kind:?} block (index {index})"
                    )));
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            let index = payload.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            match state.open.get(index).copied() {
                Some(kind @ BlockKind::Text) => {
                    events.push(StreamEvent::TextEnd { id: block_id(kind, index) });
                }
                Some(kind @ BlockKind::Thinking) => {
                    events.push(StreamEvent::ThinkingEnd { id: block_id(kind, index) });
                }
                Some(kind @ BlockKind::Tool) => {
                    events.push(StreamEvent::ToolcallEnd { id: block_id(kind, index) });
                }
                None => {}
            }
        }
        "message_delta" => {
            if let Some(reason) =
                payload.pointer("/delta/stop_reason").and_then(Value::as_str)
            {
                match map_stop_reason(ApiKind::AnthropicMessages, reason) {
                    crate::stop::StopMapping::Stop(reason) => {
                        events.push(StreamEvent::Done { reason });
                    }
                    crate::stop::StopMapping::Error(reason) => {
                        events.push(StreamEvent::Error {
                            reason,
                            message: format!("stop reason {reason:?} maps to error").into(),
                        });
                    }
                }
            }
        }
        "message_stop" => {
            // Terminal event already sent via message_delta; nothing to do.
        }
        "ping" => {}
        "error" => {
            let message = payload
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("upstream error event");
            events.push(StreamEvent::Error { reason: ErrorReason::Rejected, message: message.into() });
        }
        _ => {}
    }
    events
}

/// Auxiliary: authoritative tool-args final parse (repairing) for tests and
/// tool-result assembly.
pub fn finalize_tool_args(buffer: PartialJson) -> Value {
    buffer.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::StopReason;
    use serde_json::json;

    #[test]
    fn full_triplet_flow_with_tool() {
        let mut s = AnthropicStreamState::default();
        let ev = decode_event("message_start", &json!({"type":"message_start"}), &mut s);
        assert_eq!(ev, vec![StreamEvent::Start]);

        let ev = decode_event(
            "content_block_start",
            &json!({"type":"content_block_start","index":0,"content_block":{"type":"text"}}),
            &mut s,
        );
        assert_eq!(ev, vec![StreamEvent::TextStart { id: BlockId("text_0".into()) }]);

        let ev = decode_event(
            "content_block_delta",
            &json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}),
            &mut s,
        );
        assert_eq!(
            ev,
            vec![StreamEvent::TextDelta { id: BlockId("text_0".into()), text: "Hi".into() }]
        );

        let ev = decode_event(
            "content_block_stop",
            &json!({"type":"content_block_stop","index":0}),
            &mut s,
        );
        assert_eq!(ev, vec![StreamEvent::TextEnd { id: BlockId("text_0".into()) }]);

        let ev = decode_event(
            "content_block_start",
            &json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu_1","name":"grep"}}),
            &mut s,
        );
        assert!(matches!(ev[0], StreamEvent::ToolcallStart { ref call, .. } if call.name == "grep" && call.call_id == "tu_1"));

        let ev = decode_event(
            "content_block_delta",
            &json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"pat"}}),
            &mut s,
        );
        assert_eq!(
            ev,
            vec![StreamEvent::ToolcallDelta {
                id: BlockId("tool_1".into()),
                json: "{\"pat".into()
            }]
        );

        let ev = decode_event(
            "content_block_stop",
            &json!({"type":"content_block_stop","index":1}),
            &mut s,
        );
        assert_eq!(ev, vec![StreamEvent::ToolcallEnd { id: BlockId("tool_1".into()) }]);

        let ev = decode_event(
            "message_delta",
            &json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}),
            &mut s,
        );
        assert_eq!(ev, vec![StreamEvent::Done { reason: StopReason::ToolUse }]);
    }

    #[test]
    fn stop_reason_table_anthropic() {
        let mut s = AnthropicStreamState::default();
        let check = |wire: &str, s: &mut AnthropicStreamState| {
            decode_event(
                "message_delta",
                &json!({"type":"message_delta","delta":{"stop_reason":wire}}),
                s,
            )
            .pop()
            .expect("terminal")
        };
        assert_eq!(check("end_turn", &mut s), StreamEvent::Done { reason: StopReason::Stop });
        assert_eq!(check("max_tokens", &mut s), StreamEvent::Done { reason: StopReason::Length });
        assert_eq!(check("tool_use", &mut s), StreamEvent::Done { reason: StopReason::ToolUse });
        assert!(matches!(check("refusal", &mut s), StreamEvent::Error { .. }));
        assert!(matches!(check("bogus", &mut s), StreamEvent::Error { .. }));
    }

    #[test]
    fn thinking_blocks() {
        let mut s = AnthropicStreamState::default();
        let _ = decode_event("message_start", &json!({"type":"message_start"}), &mut s);
        let ev = decode_event(
            "content_block_start",
            &json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}),
            &mut s,
        );
        assert_eq!(ev, vec![StreamEvent::ThinkingStart { id: BlockId("thinking_0".into()) }]);
        let ev = decode_event(
            "content_block_delta",
            &json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"let me think"}}),
            &mut s,
        );
        assert_eq!(
            ev,
            vec![StreamEvent::ThinkingDelta {
                id: BlockId("thinking_0".into()),
                text: "let me think".into()
            }]
        );
        let ev = decode_event(
            "content_block_stop",
            &json!({"type":"content_block_stop","index":0}),
            &mut s,
        );
        assert_eq!(ev, vec![StreamEvent::ThinkingEnd { id: BlockId("thinking_0".into()) }]);
    }

    #[test]
    fn interleaved_blocks_keep_ids() {
        let mut s = AnthropicStreamState::default();
        let _ = decode_event(
            "content_block_start",
            &json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}),
            &mut s,
        );
        let _ = decode_event(
            "content_block_start",
            &json!({"type":"content_block_start","index":1,"content_block":{"type":"text"}}),
            &mut s,
        );
        let ev = decode_event(
            "content_block_delta",
            &json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"a"}}),
            &mut s,
        );
        assert_eq!(
            ev,
            vec![StreamEvent::TextDelta { id: BlockId("text_1".into()), text: "a".into() }]
        );
    }

    #[test]
    fn error_event_becomes_stream_error() {
        let mut s = AnthropicStreamState::default();
        let ev = decode_event(
            "error",
            &json!({"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}),
            &mut s,
        );
        assert!(matches!(
            ev[0],
            StreamEvent::Error { reason: ErrorReason::Rejected, .. }
        ));
    }

    #[test]
    fn ping_ignored_unknown_ignored() {
        let mut s = AnthropicStreamState::default();
        assert!(decode_event("ping", &json!({"type":"ping"}), &mut s).is_empty());
        assert!(decode_event("x", &json!({"type":"whatever"}), &mut s).is_empty());
    }
}
