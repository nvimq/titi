//! Session recap panel: what happened, in collapsible sections.
//!
//! The transcript shows the conversation as it streams; the recap is the
//! after-the-fact view — how many turns, which tools ran and how long they
//! took, which files were touched, what failed. Every section collapses to a
//! one-line summary and expands to its full content, and `Ctrl+O` toggles all
//! of them at once (the `Ctrl+O` of the reference TUI).
//!
//! Pure crate: render needs a width and a theme, so it is deterministic in
//! tests.

use crate::component::Component;
use crate::panels::{box_bot, box_top_title};
use crate::width::{truncate_to_width, visible_width, wrap_text_with_ansi};

/// One collapsible block of the recap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecapSection {
    /// Header shown whether or not the section is expanded.
    pub title: String,
    /// One line shown next to the title, collapsed or not.
    pub summary: String,
    /// Full content, shown only when expanded. One entry per line.
    pub lines: Vec<String>,
}

impl RecapSection {
    pub fn new(title: impl Into<String>, summary: impl Into<String>, lines: Vec<String>) -> Self {
        Self {
            title: title.into(),
            summary: summary.into(),
            lines,
        }
    }

    /// A section with nothing behind it: the header still tells the story.
    pub fn empty(title: impl Into<String>, summary: impl Into<String>) -> Self {
        Self::new(title, summary, Vec::new())
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// The recap overlay: sections, which are open, and where the cursor is.
pub struct Recap {
    sections: Vec<RecapSection>,
    expanded: Vec<bool>,
    selected: usize,
    max_visible: Option<usize>,
    closed: bool,
}

impl Recap {
    /// Opens with every section collapsed: the summary is the point, the
    /// detail is on demand.
    pub fn new(sections: Vec<RecapSection>) -> Self {
        let expanded = vec![false; sections.len()];
        Self {
            sections,
            expanded,
            selected: 0,
            max_visible: None,
            closed: false,
        }
    }

    pub fn sections(&self) -> &[RecapSection] {
        &self.sections
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn is_expanded(&self, index: usize) -> bool {
        self.expanded.get(index).copied().unwrap_or(false)
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Whether every section is open.
    pub fn all_expanded(&self) -> bool {
        !self.expanded.is_empty() && self.expanded.iter().all(|open| *open)
    }

    /// Cap painted rows (compact overlay).
    pub fn set_max_visible(&mut self, rows: usize) {
        self.max_visible = Some(rows.max(1));
    }

    /// Opens every section; collapses them all when they already are.
    pub fn toggle_all(&mut self) {
        let open = !self.all_expanded();
        for slot in &mut self.expanded {
            *slot = open;
        }
    }

    pub fn expand_all(&mut self) {
        for slot in &mut self.expanded {
            *slot = true;
        }
    }

    pub fn collapse_all(&mut self) {
        for slot in &mut self.expanded {
            *slot = false;
        }
    }

    fn toggle_selected(&mut self) {
        if let Some(slot) = self.expanded.get_mut(self.selected) {
            *slot = !*slot;
        }
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self) {
        if self.selected + 1 < self.sections.len() {
            self.selected += 1;
        }
    }

    /// The painted rows, with the section each belongs to. `None` marks a
    /// header row; `Some(index)` a body row of that section.
    fn rows(&self, width: u16) -> Vec<(Option<usize>, String)> {
        let inner = width.saturating_sub(4).max(6) as usize;
        let mut rows = Vec::new();
        for (index, section) in self.sections.iter().enumerate() {
            let marker = if self.is_expanded(index) {
                "▾"
            } else {
                "▸"
            };
            let pointer = if index == self.selected { "▶" } else { " " };
            let head = if section.summary.is_empty() {
                format!("{pointer}{marker} {}", section.title)
            } else {
                format!("{pointer}{marker} {} — {}", section.title, section.summary)
            };
            rows.push((None, truncate_to_width(&head, inner)));
            if !self.is_expanded(index) {
                continue;
            }
            for line in &section.lines {
                for wrapped in wrap_text_with_ansi(line, inner.saturating_sub(2).max(1)) {
                    rows.push((Some(index), format!("  {wrapped}")));
                }
            }
        }
        rows
    }

    /// Window of `rows` that keeps the selected section in view.
    ///
    /// The budget is the overlay's whole height, so the two border rows come
    /// out of it — otherwise the composite clips the title row off the top.
    fn window(&self, rows: &[(Option<usize>, String)]) -> std::ops::Range<usize> {
        let Some(budget) = self.max_visible else {
            return 0..rows.len();
        };
        let budget = budget.saturating_sub(2).max(1);
        // The selected section starts at the first header after the previous
        // one; anchoring on it keeps its body visible as it grows.
        let anchor = rows
            .iter()
            .enumerate()
            .filter(|(_, (owner, _))| owner.is_none())
            .nth(self.selected)
            .map_or(0, |(at, _)| at);
        let start = anchor.min(rows.len().saturating_sub(1));
        let end = (start + budget).min(rows.len());
        start..end
    }
}

impl Component for Recap {
    fn render(&mut self, width: u16) -> Vec<String> {
        if self.closed || self.sections.is_empty() {
            return Vec::new();
        }
        let w = width as usize;
        let inner_w = w.saturating_sub(4).max(6);
        let rows = self.rows(width);
        let window = self.window(&rows);

        let mut out = Vec::new();
        let title = format!("Recap  {}/{}", self.selected + 1, self.sections.len());
        out.push(box_top_title(inner_w, &title));
        for (_, row) in &rows[window] {
            let shown = truncate_to_width(row, inner_w.saturating_sub(2));
            let pad = inner_w
                .saturating_sub(2)
                .saturating_sub(visible_width(&shown));
            out.push(format!("│ {shown}{} │", " ".repeat(pad)));
        }
        out.push(box_bot(inner_w));
        out
    }

    fn invalidate(&mut self) {}

    /// Routes a decoded key. `\x0f` is `Ctrl+O`, which opens or closes every
    /// section at once.
    fn handle_input(&mut self, data: &str) {
        if self.closed {
            return;
        }
        match data {
            "\x1b" | "\x1b\x1b" => self.closed = true,
            "\x1b[A" | "k" => self.move_up(),
            "\x1b[B" | "j" => self.move_down(),
            "\x1b[5~" => {
                for _ in 0..5 {
                    self.move_up();
                }
            }
            "\x1b[6~" => {
                for _ in 0..5 {
                    self.move_down();
                }
            }
            "\x0f" | "o" => self.toggle_all(),
            "\r" | "\n" | " " => self.toggle_selected(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recap() -> Recap {
        Recap::new(vec![
            RecapSection::new("Session", "12 entries", vec!["id: abc".into()]),
            RecapSection::new("Tools", "3 calls", vec!["read ×2".into(), "bash ×1".into()]),
            RecapSection::empty("Problems", "none"),
        ])
    }

    #[test]
    fn sections_start_collapsed_and_show_their_summary() {
        let mut recap = recap();
        let rendered = recap.render(60).join("\n");
        assert!(rendered.contains("Recap  1/3"), "{rendered}");
        assert!(rendered.contains("Session — 12 entries"), "{rendered}");
        assert!(rendered.contains("Tools — 3 calls"), "{rendered}");
        // Collapsed: the body is not painted.
        assert!(!rendered.contains("read ×2"), "{rendered}");
        assert!(!recap.is_expanded(0));
    }

    #[test]
    fn enter_opens_the_selected_section_and_escape_closes_the_panel() {
        let mut recap = recap();
        recap.handle_input("\x1b[B"); // down to Tools
        assert_eq!(recap.selected(), 1);
        recap.handle_input("\r");
        assert!(recap.is_expanded(1));

        let rendered = recap.render(60).join("\n");
        assert!(rendered.contains("read ×2"), "{rendered}");
        // The collapsed neighbour stays collapsed.
        assert!(!recap.is_expanded(0));

        recap.handle_input("\x1b");
        assert!(recap.is_closed());
        assert!(recap.render(60).is_empty());
    }

    #[test]
    fn ctrl_o_opens_and_closes_every_section_at_once() {
        let mut recap = recap();
        recap.handle_input("\x0f");
        assert!(recap.all_expanded());
        let rendered = recap.render(60).join("\n");
        assert!(rendered.contains("read ×2"), "{rendered}");
        assert!(rendered.contains("id: abc"), "{rendered}");

        recap.handle_input("\x0f");
        assert!(!recap.is_expanded(0));
        assert!(!recap.is_expanded(1));
        assert!(!recap.render(60).join("\n").contains("read ×2"));
    }

    #[test]
    fn the_cursor_stops_at_both_ends() {
        let mut recap = recap();
        recap.handle_input("\x1b[A");
        assert_eq!(recap.selected(), 0, "already at the top");
        for _ in 0..10 {
            recap.handle_input("\x1b[B");
        }
        assert_eq!(recap.selected(), 2, "clamped at the last section");
    }

    #[test]
    fn a_long_section_wraps_and_stays_inside_the_box() {
        let long = "x".repeat(400);
        let mut recap = Recap::new(vec![RecapSection::new("Files", "1 file", vec![long])]);
        recap.expand_all();
        let rows = recap.render(40);
        assert!(rows.len() > 4, "the body wraps onto several rows");
        for row in &rows {
            assert!(
                visible_width(row) <= 40,
                "row exceeds the panel width: {row:?}"
            );
        }
    }

    #[test]
    fn the_window_follows_the_selected_section() {
        let mut sections = Vec::new();
        for index in 0..12 {
            sections.push(RecapSection::new(
                format!("Section {index}"),
                "summary",
                vec![format!("body {index}")],
            ));
        }
        let mut recap = Recap::new(sections);
        recap.set_max_visible(5);
        recap.expand_all();
        for _ in 0..11 {
            recap.handle_input("\x1b[B");
        }
        let rendered = recap.render(60).join("\n");
        assert!(
            rendered.contains("Section 11"),
            "the selected section is visible: {rendered}"
        );
        assert!(
            !rendered.contains("Section 0 "),
            "and the far end scrolled away: {rendered}"
        );
    }

    #[test]
    fn an_empty_recap_paints_nothing() {
        let mut recap = Recap::new(Vec::new());
        assert!(recap.render(60).is_empty());
    }
}
