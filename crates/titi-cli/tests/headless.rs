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
