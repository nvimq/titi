//! Integration test for mouse drag-select — emulated SGR-mouse events.
//!
//! Contract: `docs/research/agent-ux/README.md` (DoD mouse item):
//! drag-select draws selection background instead of SGR inverse; the SGR
//! 1006 events are decoded by [`titi_tui::input::InputBuffer`] into
//! [`titi_tui::input::InputEvent::Mouse`] and drive the selection model.

use std::collections::HashMap;
use serde_json::json;

use titi_tui::input::{InputBuffer, InputEvent, MouseKind};
use titi_tui::selection::Selection;
use titi_tui::theme::{ColorMode, SymbolPreset, Theme, ThemeBg};

fn test_theme() -> Theme {
    let mut fg = HashMap::new();
    fg.insert("error".into(), json!("#ff0000"));
    let mut bg = HashMap::new();
    bg.insert("selectedBg".into(), json!("#335599"));
    Theme::new(
        "test".into(),
        fg,
        bg,
        ColorMode::Truecolor,
        SymbolPreset::Unicode,
        HashMap::new(),
        None,
        None,
    )
    .expect("test theme builds")
}

/// Feed SGR sequences and collect the decoded Mouse events.
fn mouse_events(sequences: &[&str]) -> Vec<(MouseKind, u16, u16)> {
    let mut buf = InputBuffer::new();
    let mut out = Vec::new();
    for seq in sequences {
        for ev in buf.feed(seq.as_bytes()) {
            if let InputEvent::Mouse { kind, x, y } = ev {
                out.push((kind, x, y));
            }
        }
    }
    out
}

#[test]
fn sgr_events_decode_press_drag_release() {
    let events = mouse_events(&["\x1b[<0;10;20M", "\x1b[<32;15;22M", "\x1b[<32;18;24M", "\x1b[<0;18;24m"]);
    assert_eq!(
        events,
        vec![
            (MouseKind::Press, 10, 20),
            (MouseKind::Drag, 15, 22),
            (MouseKind::Drag, 18, 24),
            (MouseKind::Release, 18, 24),
        ]
    );
}

#[test]
fn drag_select_paints_background_over_selected_rows() {
    // Emulate: press at (2, 1), drag to (6, 3), release.
    let events = mouse_events(&["\x1b[<0;2;1M", "\x1b[<32;6;3M", "\x1b[<0;6;3m"]);
    let (press, drag, _release) = (events[0], events[1], events[2]);

    let mut selection = Selection::anchor(press.1, press.2);
    selection.drag(drag.1, drag.2);
    selection.release();

    assert_eq!(selection.rect(), Some((2, 1, 6, 3)));

    // A viewport: rows 0..4.  Selection covers rows 1..=3, cols 2..=6.
    let theme = test_theme();
    let rows: Vec<String> = (0..4)
        .map(|i| format!("row {i} — pad"), )
        .collect();
    let painted = selection.apply_background(&rows, &theme);
    let bg = theme.get_bg_ansi(ThemeBg::SelectedBg);

    // Row 0 outside the selection → untouched.
    assert!(!painted[0].contains(&bg), "row 0 untouched: {:?}", painted[0]);
    // Rows 1..=3 painted (cols 2..=6 of 11 columns).
    for i in 1..=3 {
        assert!(painted[i].contains(&bg), "row {i} painted: {:?}", painted[i]);
        assert!(painted[i].contains("\x1b[49m"), "row {i} closes bg: {:?}", painted[i]);
    }
    // The bg splits the row at column 2, but the visible text survives
    // across the split: prefix "ro" before the bg, "w 1 " inside, "pad" after.
    assert!(painted[1].starts_with("ro"), "prefix preserved: {:?}", painted[1]);
    assert!(painted[1].contains("w 1 "), "selected text preserved: {:?}", painted[1]);
    assert!(painted[3].ends_with(" pad"), "suffix preserved: {:?}", painted[3]);
}

#[test]
fn scroll_events_do_not_create_selection() {
    let events = mouse_events(&["\x1b[<64;5;5M", "\x1b[<128;5;5M"]);
    assert_eq!(events[0].0, MouseKind::ScrollUp);
    assert_eq!(events[1].0, MouseKind::ScrollDown);
    // No press → no selection anchor.
    assert!(Selection::default().rect().is_none());
}
