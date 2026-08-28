//! Overlay panels for agent UX — selection lists (model, session) and
//! approval prompts with Esc cancel-without-delete.
//!
//! Each panel implements `Component` and can be shown via
//! `OverlayStack::show`.  The application layer checks `result()` after
//! routing input and calls `hide()` on the overlay handle when the panel
//! is closed.
//!
//! Contract: `docs/research/agent-ux/README.md`.

use crate::component::Component;
use crate::width::{truncate_to_width, visible_width};

/// Result of a closed panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelResult<T> {
    /// The selected item, or `None` if cancelled.
    pub selected: Option<T>,
    /// Whether the panel was cancelled (Esc).
    pub cancelled: bool,
}

/// A generic selection-list panel.
///
/// Renders a bordered box with a title and scrollable list.  Navigation:
///
/// - `↑`/`↓` or `k`/`j` — move selection
/// - `Enter` — confirm selection
/// - `Esc` — cancel (no selection)
pub struct SelectionPanel<T> {
    title: String,
    items: Vec<T>,
    labels: Vec<String>,
    selected: usize,
    result: Option<PanelResult<T>>,
    closed: bool,
}

impl<T> SelectionPanel<T> {
    /// Create a new selection panel.
    ///
    /// `items` — the data items; `labels` — their display strings.
    /// Each item at index `i` is displayed as `labels[i]`.
    ///
    /// # Panics
    ///
    /// Panics if `items` and `labels` have different lengths.
    pub fn new(title: &str, items: Vec<T>, labels: Vec<String>) -> Self {
        assert_eq!(
            items.len(),
            labels.len(),
            "items and labels must be same length"
        );
        SelectionPanel {
            title: title.to_owned(),
            items,
            labels,
            selected: 0,
            result: None,
            closed: false,
        }
    }

    /// The result once the panel is closed, or `None` while still open.
    pub fn result(&self) -> Option<&PanelResult<T>> {
        self.result.as_ref()
    }

    /// Whether the panel has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Consume the panel and return its result, or `None` if not yet closed.
    pub fn into_result(self) -> Option<PanelResult<T>> {
        self.result
    }

    /// Selected index (0-based).
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self) {
        if !self.items.is_empty() && self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }

    fn confirm(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let item = self.items.swap_remove(self.selected);
        self.result = Some(PanelResult {
            selected: Some(item),
            cancelled: false,
        });
        self.closed = true;
    }

    fn cancel(&mut self) {
        self.result = Some(PanelResult {
            selected: None,
            cancelled: true,
        });
        self.closed = true;
    }
}

impl<T> Component for SelectionPanel<T> {
    fn render(&mut self, width: u16) -> Vec<String> {
        if self.closed {
            return Vec::new();
        }

        let w = width as usize;
        if w < 8 {
            // Too narrow for a border — plain list.
            return self
                .labels
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    if i == self.selected {
                        format!("> {label}")
                    } else {
                        format!("  {label}")
                    }
                })
                .collect();
        }

        let inner_w = w.saturating_sub(4).max(6);
        let title_w = inner_w.saturating_sub(2);

        // Title line (centred, truncated).
        let display_title = truncate_to_width(&self.title, title_w);
        let title_pad = title_w.saturating_sub(visible_width(&display_title));
        let left_pad = title_pad / 2;
        let right_pad = title_pad - left_pad;
        let title_line = format!(
            "│ {}{}{} │",
            " ".repeat(left_pad),
            display_title,
            " ".repeat(right_pad)
        );

        let mut rows = Vec::new();
        rows.push(format!("┌{}┐", "─".repeat(inner_w)));
        rows.push(title_line);
        rows.push(format!("├{}┤", "─".repeat(inner_w)));

        for (i, label) in self.labels.iter().enumerate() {
            let truncated = truncate_to_width(label, inner_w.saturating_sub(2));
            let pad = inner_w.saturating_sub(2) - visible_width(&truncated);
            let marker = if i == self.selected { "▶" } else { " " };
            rows.push(format!("│ {marker}{truncated}{} │", " ".repeat(pad)));
        }

        rows.push(format!("└{}┘", "─".repeat(inner_w)));
        rows
    }

    fn handle_input(&mut self, data: &str) {
        if self.closed {
            return;
        }
        match data {
            "\x1b" | "\x1b\x1b" => self.cancel(), // Esc
            "\x1b[A" | "k" => self.move_up(),     // Up / k
            "\x1b[B" | "j" => self.move_down(),   // Down / j
            "\r" | "\n" => self.confirm(),        // Enter
            _ => {}
        }
    }

    fn wants_key_release(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Approval panel  (Yes / No / Cancel)
// ---------------------------------------------------------------------------

/// A three-choice approval panel: Yes, No, Cancel.
pub type ApprovalPanel = SelectionPanel<&'static str>;

impl ApprovalPanel {
    /// Create a new approval panel with the given prompt.
    pub fn prompt(title: &str) -> Self {
        SelectionPanel::new(
            title,
            vec!["Yes", "No", "Cancel"],
            vec!["Yes".into(), "No".into(), "Cancel".into()],
        )
    }
}

// ---------------------------------------------------------------------------
// Session switcher  (Ctrl+X)
// ---------------------------------------------------------------------------

/// Outcome of closing the session switcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionAction {
    /// Switch to the session at the given index (Enter).
    Switch(usize),
    /// Close the session at the given index (Ctrl+D).
    Close(usize),
    /// Create a new session (Ctrl+N).
    New,
    /// Reload the list (Ctrl+R) — the panel stays open.
    Refresh,
    /// Cancelled (Esc) — no action, no deletion.
    Cancel,
}

/// Live session switcher.
///
/// Hermes-style: `↑`/`↓` move, `Enter` switches, `Ctrl+D` closes the
/// selected session, `Ctrl+N` creates a new one, `Ctrl+R` refreshes,
/// `Esc` cancels without deleting.
pub struct SessionSwitcher {
    titles: Vec<String>,
    selected: usize,
    action: Option<SessionAction>,
    closed: bool,
}

impl SessionSwitcher {
    /// Create a switcher over the given session titles (one per session).
    pub fn new(titles: Vec<String>) -> Self {
        SessionSwitcher {
            titles,
            selected: 0,
            action: None,
            closed: false,
        }
    }

    /// The chosen action once closed, or `None` while still open.
    pub fn action(&self) -> Option<&SessionAction> {
        self.action.as_ref()
    }

    /// Consume the panel and return its action, or `None` if not yet closed.
    pub fn into_action(self) -> Option<SessionAction> {
        self.action
    }

    /// Whether the switcher has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    fn close_with(&mut self, action: SessionAction) {
        self.action = Some(action);
        self.closed = true;
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self) {
        if !self.titles.is_empty() && self.selected + 1 < self.titles.len() {
            self.selected += 1;
        }
    }
}

impl Component for SessionSwitcher {
    fn render(&mut self, width: u16) -> Vec<String> {
        if self.closed {
            return Vec::new();
        }
        let w = width as usize;
        if w < 8 {
            return self
                .titles
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    if i == self.selected {
                        format!("> {t}")
                    } else {
                        format!("  {t}")
                    }
                })
                .collect();
        }

        let inner_w = w.saturating_sub(4).max(6);
        let mut rows = Vec::new();
        rows.push(format!("┌{}┐", "─".repeat(inner_w)));
        rows.push(format!(
            "│ {}{} │",
            "Sessions".to_owned(),
            " ".repeat(inner_w.saturating_sub(9))
        ));
        rows.push(format!("├{}┤", "─".repeat(inner_w)));

        for (i, title) in self.titles.iter().enumerate() {
            let truncated = truncate_to_width(title, inner_w.saturating_sub(2));
            let pad = inner_w.saturating_sub(2) - visible_width(&truncated);
            let marker = if i == self.selected { "▶" } else { " " };
            rows.push(format!("│ {marker}{truncated}{} │", " ".repeat(pad)));
        }

        rows.push(format!("└{}┘", "─".repeat(inner_w)));
        rows
    }

    fn handle_input(&mut self, data: &str) {
        if self.closed {
            return;
        }
        match data {
            "\x1b" | "\x1b\x1b" => self.close_with(SessionAction::Cancel), // Esc
            "\x1b[A" | "k" => self.move_up(),                              // Up / k
            "\x1b[B" | "j" => self.move_down(),                            // Down / j
            "\r" | "\n" => {
                if !self.titles.is_empty() {
                    self.close_with(SessionAction::Switch(self.selected));
                }
            }
            "\x04" => {
                // Ctrl+D — close selected session
                if !self.titles.is_empty() {
                    self.close_with(SessionAction::Close(self.selected));
                }
            }
            "\x0e" => self.close_with(SessionAction::New), // Ctrl+N
            "\x12" => self.action = Some(SessionAction::Refresh), // Ctrl+R, keep open
            _ => {}
        }
    }

    fn wants_key_release(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- SelectionPanel navigation ----------------------------------------

    #[test]
    fn panel_initial_state() {
        let p = SelectionPanel::new(
            "Choose",
            vec!["a", "b", "c"],
            vec!["Item A".into(), "Item B".into(), "Item C".into()],
        );
        assert!(!p.is_closed());
        assert!(p.result().is_none());
        assert_eq!(p.selected_index(), 0);
    }

    #[test]
    fn panel_arrow_navigation() {
        let mut p = SelectionPanel::new(
            "Choose",
            vec!["a", "b", "c"],
            vec!["A".into(), "B".into(), "C".into()],
        );
        assert_eq!(p.selected, 0);
        p.handle_input("\x1b[B"); // Down
        assert_eq!(p.selected, 1);
        p.handle_input("\x1b[B"); // Down
        assert_eq!(p.selected, 2);
        p.handle_input("\x1b[B"); // Down (at bottom, no-op)
        assert_eq!(p.selected, 2);
        p.handle_input("\x1b[A"); // Up
        assert_eq!(p.selected, 1);
        p.handle_input("\x1b[A"); // Up
        assert_eq!(p.selected, 0);
        p.handle_input("\x1b[A"); // Up (at top, no-op)
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn panel_enter_confirms() {
        let mut p = SelectionPanel::new(
            "Choose",
            vec!["apple", "banana", "cherry"],
            vec!["Apple".into(), "Banana".into(), "Cherry".into()],
        );
        p.handle_input("\x1b[B"); // move to banana
        p.handle_input("\x1b[B"); // move to cherry
        p.handle_input("\r"); // Enter
        assert!(p.is_closed());
        let res = p.result().unwrap();
        assert!(!res.cancelled);
        assert_eq!(res.selected, Some("cherry"));
    }

    #[test]
    fn panel_esc_cancels() {
        let mut p = SelectionPanel::new(
            "Choose",
            vec!["a", "b"],
            vec!["A".into(), "B".into()],
        );
        p.handle_input("\x1b"); // Esc
        assert!(p.is_closed());
        let res = p.result().unwrap();
        assert!(res.cancelled);
        assert_eq!(res.selected, None);
    }

    #[test]
    fn panel_vim_keys_work() {
        let mut p = SelectionPanel::new(
            "X",
            vec!["a", "b", "c"],
            vec!["a".into(), "b".into(), "c".into()],
        );
        p.handle_input("j");
        assert_eq!(p.selected, 1);
        p.handle_input("j");
        assert_eq!(p.selected, 2);
        p.handle_input("k");
        assert_eq!(p.selected, 1);
    }

    #[test]
    fn panel_into_result() {
        let mut p = SelectionPanel::new("X", vec!["only"], vec!["Only".into()]);
        p.handle_input("\r");
        let res = p.into_result().unwrap();
        assert_eq!(res.selected, Some("only"));
    }

    // ---- ApprovalPanel ----------------------------------------------------

    #[test]
    fn approval_panel_yes() {
        let mut p = ApprovalPanel::prompt("Proceed?");
        p.handle_input("\r"); // Yes is default (index 0)
        assert!(p.is_closed());
        let res = p.result().unwrap();
        assert_eq!(res.selected, Some("Yes"));
    }

    #[test]
    fn approval_panel_no() {
        let mut p = ApprovalPanel::prompt("Proceed?");
        p.handle_input("\x1b[B"); // No
        p.handle_input("\r");
        let res = p.result().unwrap();
        assert_eq!(res.selected, Some("No"));
    }

    #[test]
    fn approval_panel_cancel() {
        let mut p = ApprovalPanel::prompt("Proceed?");
        p.handle_input("\x1b"); // Esc
        assert!(p.result().unwrap().cancelled);
    }

    // ---- Render -----------------------------------------------------------

    #[test]
    fn render_closed_panel_is_empty() {
        let mut p = SelectionPanel::<&str>::new("X", vec![], vec![]);
        p.handle_input("\x1b");
        assert!(p.render(80).is_empty());
    }

    #[test]
    fn render_narrow_panel_uses_simple_format() {
        let mut p = SelectionPanel::new("X", vec!["a", "b"], vec!["A".into(), "B".into()]);
        let rows = p.render(4);
        assert!(rows[0].contains("> A"), "narrow render: {rows:?}");
        assert!(rows[1].contains("  B"), "narrow render: {rows:?}");
    }

    #[test]
    fn render_wide_panel_has_border() {
        let mut p = SelectionPanel::new("Choose", vec!["x"], vec!["Item".into()]);
        let rows = p.render(40);
        assert!(rows[0].starts_with('┌'), "should start with top border: {rows:?}");
        assert!(
            rows.last().unwrap().starts_with('└'),
            "should end with bottom border"
        );
    }

    // ---- SessionSwitcher --------------------------------------------------

    #[test]
    fn switcher_enter_switches() {
        let mut s = SessionSwitcher::new(vec!["A".into(), "B".into()]);
        s.handle_input("\x1b[B"); // move to B
        s.handle_input("\r"); // Enter
        assert_eq!(s.action(), Some(&SessionAction::Switch(1)));
    }

    #[test]
    fn switcher_ctrl_d_closes() {
        let mut s = SessionSwitcher::new(vec!["A".into(), "B".into(), "C".into()]);
        s.handle_input("\x1b[B"); // move to index 1
        s.handle_input("\x04"); // Ctrl+D
        assert_eq!(s.action(), Some(&SessionAction::Close(1)));
    }

    #[test]
    fn switcher_ctrl_n_new() {
        let mut s = SessionSwitcher::new(vec!["A".into()]);
        s.handle_input("\x0e"); // Ctrl+N
        assert_eq!(s.action(), Some(&SessionAction::New));
    }

    #[test]
    fn switcher_ctrl_r_refreshes_keeps_open() {
        let mut s = SessionSwitcher::new(vec!["A".into(), "B".into()]);
        s.handle_input("\x12"); // Ctrl+R
        assert_eq!(s.action(), Some(&SessionAction::Refresh));
        assert!(!s.is_closed(), "refresh must keep the switcher open");
    }

    #[test]
    fn switcher_esc_cancels_without_delete() {
        let mut s = SessionSwitcher::new(vec!["A".into(), "B".into()]);
        s.handle_input("\x1b"); // Esc
        assert_eq!(s.action(), Some(&SessionAction::Cancel));
        assert!(s.is_closed());
    }

    #[test]
    fn switcher_empty_list_enter_is_noop() {
        let mut s = SessionSwitcher::new(vec![]);
        s.handle_input("\r");
        assert!(!s.is_closed(), "Enter on empty list must not close");
    }

    #[test]
    fn switcher_into_action() {
        let mut s = SessionSwitcher::new(vec!["Only".into()]);
        s.handle_input("\x0e"); // Ctrl+N
        assert_eq!(s.into_action(), Some(SessionAction::New));
    }
}
