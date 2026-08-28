//! Per-family stop-reason mapping tables (omp provider-streaming tables).

use crate::stream::{ErrorReason, StopReason};
use crate::transport::ApiKind;

/// Result of mapping a wire stop reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopMapping {
    /// Mapped onto the normalized [`StopReason`].
    Stop(StopReason),
    /// Safety/malformed cases: the turn ends as a stream error.
    Error(ErrorReason),
}

impl StopMapping {
    /// Unwrap to a plain stop reason; error-mappings collapse to
    /// [`StopReason::Stop`] is never done implicitly — use `as_stop`.
    pub fn as_stop(self) -> Option<StopReason> {
        match self {
            StopMapping::Stop(r) => Some(r),
            StopMapping::Error(_) => None,
        }
    }
}

/// Map a family-specific wire stop reason onto the normalized contract.
///
/// Tables (omp://provider-streaming-internals.md):
/// - Anthropic: `end_turn`→Stop, `max_tokens`→Length, `tool_use`→ToolUse,
///   `stop_sequence`→Stop, safety/refusal→Error.
/// - OpenAI Responses: `completed`→Stop, `incomplete`→Length,
///   `failed`/`cancelled`→Error.
/// - OpenAI Completions: `stop`→Stop, `length`→Length,
///   `tool_calls`/`function_call`→ToolUse.
/// - Gemini: `STOP`→Stop, `MAX_TOKENS`→Length, `SAFETY`/`RECITATION`/
///   `MALFORMED_FUNCTION_CALL`/`PROHIBITED_CONTENT`/`BLOCKLIST`→Error.
pub fn map_stop_reason(family: ApiKind, wire: &str) -> StopMapping {
    match family {
        ApiKind::AnthropicMessages => match wire {
            "end_turn" | "stop_sequence" => StopMapping::Stop(StopReason::Stop),
            "max_tokens" => StopMapping::Stop(StopReason::Length),
            "tool_use" => StopMapping::Stop(StopReason::ToolUse),
            "pause_turn" | "refusal" => {
                StopMapping::Error(ErrorReason::Connection)
            }
            _ => StopMapping::Error(ErrorReason::Malformed),
        },
        ApiKind::OpenAiResponses => match wire {
            "completed" => StopMapping::Stop(StopReason::Stop),
            "incomplete" => StopMapping::Stop(StopReason::Length),
            "failed" | "cancelled" => StopMapping::Error(ErrorReason::Rejected),
            _ => StopMapping::Error(ErrorReason::Malformed),
        },
        ApiKind::OpenAiCompletions => match wire {
            "stop" => StopMapping::Stop(StopReason::Stop),
            "length" => StopMapping::Stop(StopReason::Length),
            "tool_calls" | "function_call" => StopMapping::Stop(StopReason::ToolUse),
            _ => StopMapping::Error(ErrorReason::Malformed),
        },
        ApiKind::GeminiGenerateContent => match wire {
            "STOP" => StopMapping::Stop(StopReason::Stop),
            "MAX_TOKENS" => StopMapping::Stop(StopReason::Length),
            "SAFETY" | "RECITATION" | "PROHIBITED_CONTENT" | "BLOCKLIST" => {
                StopMapping::Error(ErrorReason::Rejected)
            }
            "MALFORMED_FUNCTION_CALL" => StopMapping::Error(ErrorReason::Malformed),
            _ => StopMapping::Error(ErrorReason::Malformed),
        },
    }
}

/// Chat Completions quirk: a bare `finish_reason: "stop"` is raised to
/// `ToolUse` when structural tool blocks were seen during the turn.
pub fn promote_stop_for_tools(seen_tools: bool, reason: StopReason) -> StopReason {
    if seen_tools && reason == StopReason::Stop {
        StopReason::ToolUse
    } else {
        reason
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_table() {
        use ApiKind::AnthropicMessages as A;
        assert_eq!(map_stop_reason(A, "end_turn"), StopMapping::Stop(StopReason::Stop));
        assert_eq!(map_stop_reason(A, "stop_sequence"), StopMapping::Stop(StopReason::Stop));
        assert_eq!(map_stop_reason(A, "max_tokens"), StopMapping::Stop(StopReason::Length));
        assert_eq!(map_stop_reason(A, "tool_use"), StopMapping::Stop(StopReason::ToolUse));
        assert_eq!(
            map_stop_reason(A, "refusal"),
            StopMapping::Error(ErrorReason::Connection)
        );
        assert_eq!(
            map_stop_reason(A, "who-knows"),
            StopMapping::Error(ErrorReason::Malformed)
        );
    }

    #[test]
    fn openai_responses_table() {
        use ApiKind::OpenAiResponses as R;
        assert_eq!(map_stop_reason(R, "completed"), StopMapping::Stop(StopReason::Stop));
        assert_eq!(map_stop_reason(R, "incomplete"), StopMapping::Stop(StopReason::Length));
        assert_eq!(
            map_stop_reason(R, "failed"),
            StopMapping::Error(ErrorReason::Rejected)
        );
        assert_eq!(
            map_stop_reason(R, "cancelled"),
            StopMapping::Error(ErrorReason::Rejected)
        );
        assert_eq!(
            map_stop_reason(R, "other"),
            StopMapping::Error(ErrorReason::Malformed)
        );
    }

    #[test]
    fn openai_completions_table() {
        use ApiKind::OpenAiCompletions as C;
        assert_eq!(map_stop_reason(C, "stop"), StopMapping::Stop(StopReason::Stop));
        assert_eq!(map_stop_reason(C, "length"), StopMapping::Stop(StopReason::Length));
        assert_eq!(map_stop_reason(C, "tool_calls"), StopMapping::Stop(StopReason::ToolUse));
        assert_eq!(
            map_stop_reason(C, "function_call"),
            StopMapping::Stop(StopReason::ToolUse)
        );
        assert_eq!(
            map_stop_reason(C, "content_filter"),
            StopMapping::Error(ErrorReason::Malformed)
        );
    }

    #[test]
    fn gemini_table() {
        use ApiKind::GeminiGenerateContent as G;
        assert_eq!(map_stop_reason(G, "STOP"), StopMapping::Stop(StopReason::Stop));
        assert_eq!(map_stop_reason(G, "MAX_TOKENS"), StopMapping::Stop(StopReason::Length));
        assert_eq!(map_stop_reason(G, "SAFETY"), StopMapping::Error(ErrorReason::Rejected));
        assert_eq!(
            map_stop_reason(G, "RECITATION"),
            StopMapping::Error(ErrorReason::Rejected)
        );
        assert_eq!(
            map_stop_reason(G, "MALFORMED_FUNCTION_CALL"),
            StopMapping::Error(ErrorReason::Malformed)
        );
        assert_eq!(map_stop_reason(G, "OTHER"), StopMapping::Error(ErrorReason::Malformed));
    }

    #[test]
    fn as_stop_only_for_stop_branch() {
        assert_eq!(StopMapping::Stop(StopReason::ToolUse).as_stop(), Some(StopReason::ToolUse));
        assert_eq!(StopMapping::Error(ErrorReason::Rejected).as_stop(), None);
    }
}
