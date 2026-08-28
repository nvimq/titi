//! Integration test for transcript accordion integration in titi-cli.
//!
//! Contract: `docs/research/agent-ux/README.md` (DoD transcript item):
//! thinking/tools expanded, subagents collapsed, activity hidden by default;
//! `/details <section> <mode>` switches visibility; floating-alert backstop
//! when all sections are hidden.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use titi_cli::app::{default_theme, App};
use titi_tui::markdown::Section;
use titi_tui::theme::Theme;

fn test_theme() -> Arc<Theme> {
    default_theme()
}

#[test]
fn transcript_defaults_render_accordion() {
    let mut app = App::new(
        Arc::new(AtomicBool::new(false)),
        vec!["titi".to_owned()],
        test_theme(),
    );
    app.push_transcript(Section::Thinking, "reasoning step");
    app.push_transcript(Section::Tools, "tool ran");
    app.push_transcript(Section::Subagents, "agent report");
    app.push_transcript(Section::Activity, "ambient");

    let rows = app.render();

    // thinking + tools expanded → header + body; subagents collapsed →
    // header only; activity hidden → nothing.
    let text = rows.join("\n");
    assert!(text.contains("▾ thinking"), "thinking header: {text}");
    assert!(text.contains("reasoning step"), "thinking body: {text}");
    assert!(text.contains("▾ tools"), "tools header: {text}");
    assert!(text.contains("tool ran"), "tools body: {text}");
    assert!(text.contains("▸ subagents"), "subagents collapsed: {text}");
    assert!(!text.contains("agent report"), "subagents body hidden: {text}");
    assert!(!text.contains("ambient"), "activity hidden: {text}");
    assert!(!text.contains("activity"), "no activity header: {text}");
}

#[test]
fn details_directive_expands_collapsed_section() {
    let mut app = App::new(
        Arc::new(AtomicBool::new(false)),
        vec!["titi".to_owned()],
        test_theme(),
    );
    app.push_transcript(Section::Subagents, "agent report");

    // Default: collapsed → body not rendered.
    let rows = app.render();
    let text = rows.join("\n");
    assert!(!text.contains("agent report"), "default collapsed: {text}");

    // /details subagents expanded → body renders.
    assert!(app.details("subagents expanded"));
    let rows = app.render();
    let text = rows.join("\n");
    assert!(text.contains("agent report"), "after expand: {text}");
    assert!(text.contains("▾ subagents"), "chevron updated: {text}");
}

#[test]
fn all_hidden_shows_floating_alert_backstop() {
    let mut app = App::new(
        Arc::new(AtomicBool::new(false)),
        vec!["titi".to_owned()],
        test_theme(),
    );
    app.push_transcript(Section::Thinking, "hidden thought");

    // Hide everything; activity is hidden by default.
    assert!(app.details("thinking hidden"));
    assert!(app.details("tools hidden"));
    assert!(app.details("subagents hidden"));
    assert!(app.all_hidden());

    // Backstop: the transcript renders the alert instead of nothing.
    let rows = app.render();
    let text = rows.join("\n");
    assert!(text.contains("all sections hidden"), "alert backstop: {text}");
    assert!(!text.contains("hidden thought"), "content suppressed: {text}");
}

#[test]
fn first_frame_still_queues_input_with_transcript() {
    let mut app = App::new(
        Arc::new(AtomicBool::new(false)),
        vec!["titi v0.1.0".to_owned(), String::new()],
        test_theme(),
    );
    let rows = app.render();
    assert!(rows[0].contains("titi v0.1.0"), "banner: {rows:?}");
    assert!(rows.iter().any(|r| r.contains("starting")), "status: {rows:?}");
    assert!(app.time_to_first_frame().as_millis() < 150, "ttff too slow");

    use titi_cli::first_frame::SubmitOutcome;
    assert_eq!(app.submit("queued prompt".to_owned()), SubmitOutcome::Queued);
    assert_eq!(app.queue_len(), 1);
}
