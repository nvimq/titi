use titi_engine::{EngineCommand, EngineEvent, TurnId};
use titi_providers::StopReason;

#[test]
fn engine_events_round_trip_as_jsonl() {
    let event = EngineEvent::TurnFinished {
        turn_id: TurnId(1),
        reason: StopReason::Stop,
    };
    let line = serde_json::to_string(&event).unwrap();
    let parsed: EngineEvent = serde_json::from_str(&line).unwrap();
    assert_eq!(parsed, event);

    let command = EngineCommand::SubmitPrompt { text: "hi".into() };
    let frame = serde_json::json!({ "command": command });
    let decoded: EngineCommand = serde_json::from_value(frame["command"].clone()).unwrap();
    assert_eq!(decoded, command);
}

#[test]
fn frame_version_is_optional_and_enforced() {
    use titi_cli::headless::{FrameError, RPC_PROTOCOL, decode};

    // Omitted version means "current".
    let bare = decode(r#"{"command":"Cancel"}"#).unwrap();
    assert_eq!(bare.v, None);
    assert!(matches!(bare.command, EngineCommand::Cancel));

    // The current version is accepted explicitly.
    let pinned = decode(&format!(r#"{{"v":{RPC_PROTOCOL},"command":"Cancel"}}"#)).unwrap();
    assert_eq!(pinned.v, Some(RPC_PROTOCOL));

    // A future version is refused with a message naming both sides.
    let err = decode(r#"{"v":99,"command":"Cancel"}"#).unwrap_err();
    assert_eq!(
        err,
        FrameError::UnsupportedVersion {
            client: 99,
            runner: RPC_PROTOCOL
        }
    );
    assert!(err.message().contains("99"));
    assert!(err.message().contains(&RPC_PROTOCOL.to_string()));

    // A line that is not a frame is malformed, not a version error.
    assert!(matches!(decode("not json"), Err(FrameError::Malformed(_))));
}
