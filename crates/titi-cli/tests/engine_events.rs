use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use titi_cli::app::{default_theme, App};
use titi_engine::{AgentKind, AgentStatus, EngineEvent, TurnId};
use titi_providers::StopReason;

fn app() -> App {
    App::new(
        Arc::new(AtomicBool::new(true)),
        vec!["titi".to_owned()],
        default_theme().unwrap(),
    )
}

#[test]
fn engine_events_render_stream_thinking_tools_and_agents() {
    let mut app = app();
    let turn_id = TurnId(1);
    app.ingest_engine_event(EngineEvent::TurnStarted {
        turn_id,
        model: "test/model".into(),
    });
    app.ingest_engine_event(EngineEvent::StreamDelta {
        turn_id,
        text: "answer".into(),
    });
    app.ingest_engine_event(EngineEvent::ThinkingDelta {
        turn_id,
        text: "reasoning".into(),
    });
    app.ingest_engine_event(EngineEvent::ToolStarted {
        turn_id,
        call_id: "call-1".into(),
        name: "read".into(),
    });
    app.ingest_engine_event(EngineEvent::AgentStarted {
        agent_id: "agent-1".into(),
        name: "Trace runtime".into(),
        parent_id: None,
        kind: AgentKind::Subagent,
    });
    app.ingest_engine_event(EngineEvent::AgentProgress {
        agent_id: "agent-1".into(),
        text: "reading providers".into(),
    });
    app.ingest_engine_event(EngineEvent::AgentStatusChanged {
        agent_id: "agent-1".into(),
        status: AgentStatus::Parked,
    });
    app.ingest_engine_event(EngineEvent::TurnFinished {
        turn_id,
        reason: StopReason::Stop,
    });

    let rendered = app.render().join("\n");
    assert!(rendered.contains("answer"), "{rendered}");
    assert!(rendered.contains("reasoning"), "{rendered}");
    assert!(rendered.contains("read · call-1 · running"), "{rendered}");
    assert_eq!(app.hub_peers().len(), 1);
    assert_eq!(
        app.hub_peers()[0].status,
        titi_tui::hub::AgentStatus::Parked
    );
}

#[test]
fn agents_slash_opens_live_hub() {
    let mut app = app();
    app.ingest_engine_event(EngineEvent::AgentStarted {
        agent_id: "agent-1".into(),
        name: "Worker".into(),
        parent_id: None,
        kind: AgentKind::Subagent,
    });
    let mut input = "/agents".to_owned();
    app.handle_canonical("enter", &mut input);
    let rendered = app.render().join("\n");
    assert!(rendered.contains("Agent Hub"), "{rendered}");
    assert!(rendered.contains("Worker"), "{rendered}");
}

#[test]
fn hub_r_and_x_emit_engine_commands() {
    let mut app = app();
    app.ingest_engine_event(EngineEvent::AgentStarted {
        agent_id: "agent-1".into(),
        name: "Worker".into(),
        parent_id: None,
        kind: AgentKind::Subagent,
    });
    app.ingest_engine_event(EngineEvent::AgentStatusChanged {
        agent_id: "agent-1".into(),
        status: AgentStatus::Parked,
    });
    let mut input = "/agents".to_owned();
    app.handle_canonical("enter", &mut input);
    assert_eq!(
        app.overlay_input("r"),
        Some(titi_cli::app::OverlayOutcome::HubRevive("agent-1".into()))
    );
    assert!(app.overlay_open());
    assert_eq!(
        app.overlay_input("x"),
        Some(titi_cli::app::OverlayOutcome::HubStop("agent-1".into()))
    );
}
