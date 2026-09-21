//! Integration tests for the session checkpoint surface: `/checkpoint`,
//! `/checkpoints`, `/rewind`.
//!
//! Contract: `docs/research/empryo-port/README.md` (E2 — checkpoints).

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use titi_cli::app::{App, checkpoint_session, default_theme, list_checkpoints, rewind_session};
use titi_core::session::{Role, SessionMeta, SessionStore};
use titi_tui::slash::Route;

fn app() -> App {
    App::new(
        Arc::new(AtomicBool::new(false)),
        vec!["titi".to_owned()],
        default_theme().unwrap(),
    )
}

#[test]
fn checkpoint_builtins_are_reserved() {
    let app = app();
    for name in ["checkpoint", "checkpoints", "rewind"] {
        assert_eq!(
            app.route_slash(&format!("/{name}")),
            Route::Builtin(name.to_owned())
        );
    }
    // Arguments do not change the route.
    assert_eq!(
        app.route_slash("/rewind 2"),
        Route::Builtin("rewind".to_owned())
    );
}

#[test]
fn checkpoint_rewind_roundtrip_through_the_helpers() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_dir = tmp.path();
    let store = SessionStore::new(agent_dir).unwrap();
    let sid = store.create(SessionMeta::default()).unwrap();
    store.append(&sid, Role::User, "first").unwrap();

    // The summary starts with the entry count. Inside a git checkout it also
    // names the commit the workspace was pinned to, which this run is.
    let summary = checkpoint_session(agent_dir, &sid).unwrap();
    assert!(summary.starts_with("checkpoint: 1 entries"), "{summary}");
    store.append(&sid, Role::Assistant, "second").unwrap();
    assert_eq!(store.open(&sid).unwrap().len(), 2);

    assert!(
        list_checkpoints(agent_dir, &sid)
            .unwrap()
            .contains("#1 · 1 entries")
    );
    assert!(
        rewind_session(agent_dir, &sid, None)
            .unwrap()
            .contains("rewound to checkpoint #1")
    );
    assert_eq!(store.open(&sid).unwrap().len(), 1);
}

#[test]
fn rewind_reports_missing_and_out_of_range_checkpoints() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_dir = tmp.path();
    let store = SessionStore::new(agent_dir).unwrap();
    let sid = store.create(SessionMeta::default()).unwrap();
    store.append(&sid, Role::User, "only").unwrap();

    assert_eq!(
        list_checkpoints(agent_dir, &sid).unwrap(),
        "checkpoints: none"
    );
    assert!(rewind_session(agent_dir, &sid, None).is_err());
    assert!(rewind_session(agent_dir, &sid, Some(1)).is_err());

    checkpoint_session(agent_dir, &sid).unwrap();
    // Checkpoints are 1-based, so 0 and 2 are both rejected.
    assert!(rewind_session(agent_dir, &sid, Some(0)).is_err());
    assert!(rewind_session(agent_dir, &sid, Some(2)).is_err());
    assert!(rewind_session(agent_dir, &sid, Some(1)).is_ok());
}

#[test]
fn an_app_without_a_session_reports_it_instead_of_panicking() {
    let mut app = app();
    assert_eq!(app.session_id(), None);
    app.set_session_id("live");
    assert_eq!(app.session_id(), Some("live"));
}
