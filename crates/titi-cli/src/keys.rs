//! Crossterm `KeyEvent` → canonical key id (`ctrl+q`, `alt+up`, `shift+tab`).
//!
//! Feeds [`titi_tui::keybindings::KeybindingsManager::matches_canonical`].

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use titi_tui::keys::canonical_key_id;

/// Convert a crossterm key event into a canonical key id.
///
/// Returns `None` for key-release events (OMP filters those unless a
/// component sets `wantsKeyRelease`).
pub fn canonical_from_key_event(key: &KeyEvent) -> Option<String> {
    if key.kind == KeyEventKind::Release {
        return None;
    }

    let mut parts: Vec<&str> = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl");
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("shift");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("alt");
    }
    if key.modifiers.contains(KeyModifiers::SUPER) {
        parts.push("super");
    }

    let base = match key.code {
        KeyCode::Char(' ') => "space".to_owned(),
        KeyCode::Char(c) => {
            let lower = c.to_ascii_lowercase();
            if c.is_ascii_uppercase() && !parts.contains(&"shift") {
                parts.push("shift");
            }
            lower.to_string()
        }
        KeyCode::Enter => "enter".to_owned(),
        KeyCode::Esc => "escape".to_owned(),
        KeyCode::Tab => "tab".to_owned(),
        KeyCode::BackTab => {
            if !parts.contains(&"shift") {
                parts.push("shift");
            }
            "tab".to_owned()
        }
        KeyCode::Backspace => "backspace".to_owned(),
        KeyCode::Delete => "delete".to_owned(),
        KeyCode::Up => "up".to_owned(),
        KeyCode::Down => "down".to_owned(),
        KeyCode::Left => "left".to_owned(),
        KeyCode::Right => "right".to_owned(),
        KeyCode::Home => "home".to_owned(),
        KeyCode::End => "end".to_owned(),
        KeyCode::PageUp => "pageup".to_owned(),
        KeyCode::PageDown => "pagedown".to_owned(),
        KeyCode::Insert => "insert".to_owned(),
        _ => return None,
    };

    let raw = if parts.is_empty() {
        base
    } else {
        format!("{}+{base}", parts.join("+"))
    };
    Some(canonical_key_id(&raw))
}

/// Map a key event to an overlay input sequence.
///
/// Panels consume raw decoded input (Esc, arrows, Enter, Ctrl+D/N/R, and
/// printable type-to-filter characters). Keys without a mapping are ignored
/// while an overlay is open (modal).
pub fn overlay_key_data(key: &KeyEvent) -> Option<String> {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => Some("\x1b".into()),
        (KeyCode::Enter, _) => Some("\r".into()),
        (KeyCode::Tab, _) => Some("\t".into()),
        (KeyCode::Up, _) => Some("\x1b[A".into()),
        (KeyCode::Down, _) => Some("\x1b[B".into()),
        (KeyCode::Left, _) => Some("\x1b[D".into()),
        (KeyCode::Backspace, _) => Some("\x7f".into()),
        (KeyCode::Char('d'), m) if m.contains(KeyModifiers::CONTROL) => Some("\x04".into()),
        (KeyCode::Char('n'), m) if m.contains(KeyModifiers::CONTROL) => Some("\x0e".into()),
        (KeyCode::Char('r'), m) if m.contains(KeyModifiers::CONTROL) => Some("\x12".into()),
        (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => Some("\x03".into()),
        (KeyCode::Char(c), m)
            if !m.contains(KeyModifiers::CONTROL) && !m.contains(KeyModifiers::ALT) =>
        {
            Some(c.to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn ctrl_q_follow_up() {
        assert_eq!(
            canonical_from_key_event(&key(KeyCode::Char('q'), KeyModifiers::CONTROL)).as_deref(),
            Some("ctrl+q")
        );
    }

    #[test]
    fn ctrl_enter() {
        assert_eq!(
            canonical_from_key_event(&key(KeyCode::Enter, KeyModifiers::CONTROL)).as_deref(),
            Some("ctrl+enter")
        );
    }

    #[test]
    fn alt_up_and_shift_up() {
        assert_eq!(
            canonical_from_key_event(&key(KeyCode::Up, KeyModifiers::ALT)).as_deref(),
            Some("alt+up")
        );
        assert_eq!(
            canonical_from_key_event(&key(KeyCode::Up, KeyModifiers::SHIFT)).as_deref(),
            Some("shift+up")
        );
    }

    #[test]
    fn alt_m_model_select() {
        assert_eq!(
            canonical_from_key_event(&key(KeyCode::Char('m'), KeyModifiers::ALT)).as_deref(),
            Some("alt+m")
        );
    }

    #[test]
    fn shift_tab() {
        assert_eq!(
            canonical_from_key_event(&key(KeyCode::BackTab, KeyModifiers::SHIFT)).as_deref(),
            Some("shift+tab")
        );
        assert_eq!(
            canonical_from_key_event(&key(KeyCode::BackTab, KeyModifiers::NONE)).as_deref(),
            Some("shift+tab")
        );
    }

    #[test]
    fn release_is_ignored() {
        let mut ev = key(KeyCode::Char('c'), KeyModifiers::CONTROL);
        ev.kind = KeyEventKind::Release;
        assert_eq!(canonical_from_key_event(&ev), None);
    }

    #[test]
    fn overlay_printable_reaches_filter() {
        assert_eq!(
            overlay_key_data(&key(KeyCode::Char('g'), KeyModifiers::NONE)).as_deref(),
            Some("g")
        );
        let bs = overlay_key_data(&key(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(bs.as_deref(), Some(""));
        assert_eq!(
            overlay_key_data(&key(KeyCode::Tab, KeyModifiers::NONE)).as_deref(),
            Some("\t")
        );
        assert_eq!(
            overlay_key_data(&key(KeyCode::Left, KeyModifiers::NONE)).as_deref(),
            Some("\x1b[D")
        );
    }
}
