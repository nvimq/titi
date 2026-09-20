//! Transcript component — accordion sections, per-section visibility,
//! floating-alert backstop.
//!
//! Contract: `docs/research/agent-ux/README.md` (DoD transcript item):
//! thinking and tools expanded, subagents collapsed, activity hidden by
//! default; `/details <section> <mode>` switches visibility; when every
//! section is hidden a floating alert surfaces instead of silence.
//!
//! Pure crate (no terminal I/O, no global state): render needs a width and
//! a theme so it is deterministic in tests.

use crate::markdown::{Section, SectionMode, SectionVisibility, render_markdown};
use crate::theme::{Theme, ThemeColor};
use crate::width::truncate_to_width;

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

/// A transcript entry: which section it belongs to and its markdown body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub section: Section,
    pub text: String,
}

impl Entry {
    pub fn new(section: Section, text: impl Into<String>) -> Self {
        Entry {
            section,
            text: text.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Alert
// ---------------------------------------------------------------------------

/// Floating alert shown when every section is hidden (backstop).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub text: String,
}

// ---------------------------------------------------------------------------
// Transcript
// ---------------------------------------------------------------------------

/// The transcript: ordered entries plus per-section visibility.
pub struct Transcript {
    entries: Vec<Entry>,
    visibility: SectionVisibility,
    alert: Option<Alert>,
}

impl Transcript {
    /// Create an empty transcript with the DoD default visibility
    /// (thinking/tools expanded, subagents collapsed, activity hidden).
    pub fn new() -> Self {
        Transcript {
            entries: Vec::new(),
            visibility: SectionVisibility::default(),
            alert: None,
        }
    }

    /// Append an entry.
    pub fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    /// Set the floating alert (replaces any previous).
    pub fn set_alert(&mut self, alert: Alert) {
        self.alert = Some(alert);
    }

    /// Clear the floating alert.
    pub fn clear_alert(&mut self) {
        self.alert = None;
    }

    /// The current floating alert, if any.
    pub fn alert(&self) -> Option<&Alert> {
        self.alert.as_ref()
    }

    /// Apply a `/details` directive: `"<section> <mode>"` or a global
    /// `"<mode>"` (hidden|collapsed|expanded|cycle) applied to every section.
    ///
    /// Returns `true` if visibility changed.
    pub fn details(&mut self, directive: &str) -> bool {
        let mut parts = directive.split_whitespace();
        let Some(first) = parts.next() else {
            return false;
        };
        match parts.next() {
            Some(mode) => {
                let Some(section) = Section::parse(first) else {
                    return false;
                };
                self.visibility.apply(section, mode)
            }
            None => {
                // Global: `/details cycle` / `/details hidden` / …
                let mut changed = false;
                for section in [
                    Section::Thinking,
                    Section::Tools,
                    Section::Subagents,
                    Section::Activity,
                ] {
                    changed |= self.visibility.apply(section, first);
                }
                changed
            }
        }
    }

    /// Whether every section is hidden — the app should surface the alert.
    pub fn all_hidden(&self) -> bool {
        self.visibility.all_hidden()
    }

    /// Number of entries in a section.
    pub fn count(&self, section: Section) -> usize {
        self.entries.iter().filter(|e| e.section == section).count()
    }

    /// Visibility of a section.
    pub fn mode(&self, section: Section) -> SectionMode {
        self.visibility.get(section)
    }

    /// Render the transcript at `width` using `theme`.
    ///
    /// Returns one row per terminal line.  ANSI escapes are embedded.
    pub fn render(&self, width: u16, theme: &Theme) -> Vec<String> {
        let w = width as usize;
        let mut rows = Vec::new();

        // Floating alert backstop: when every section is hidden, surface the
        // alert instead of rendering nothing.
        if self.all_hidden() {
            if let Some(alert) = &self.alert {
                rows.push(theme.fg(ThemeColor::Error, &truncate_to_width(&alert.text, w)));
            }
            return rows;
        }

        for section in [
            Section::Thinking,
            Section::Tools,
            Section::Subagents,
            Section::Activity,
        ] {
            match self.visibility.get(section) {
                SectionMode::Hidden => {}
                SectionMode::Collapsed => {
                    rows.push(section_header(
                        section,
                        self.count(section),
                        &self.visibility,
                        theme,
                        w,
                    ));
                }
                SectionMode::Expanded => {
                    rows.push(section_header(
                        section,
                        self.count(section),
                        &self.visibility,
                        theme,
                        w,
                    ));
                    for entry in self.entries.iter().filter(|e| e.section == section) {
                        rows.extend(render_markdown(&entry.text, theme, width));
                    }
                }
            }
        }
        rows
    }
}

impl Default for Transcript {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Rendered section header with chevron, name, and count.
fn section_header(
    section: Section,
    count: usize,
    visibility: &SectionVisibility,
    theme: &Theme,
    width: usize,
) -> String {
    let name = match section {
        Section::Thinking => "thinking",
        Section::Tools => "tools",
        Section::Subagents => "subagents",
        Section::Activity => "activity",
    };
    let chevron = match visibility.get(section) {
        SectionMode::Expanded => "▾",
        SectionMode::Collapsed | SectionMode::Hidden => "▸",
    };
    let label = if count == 0 {
        format!("{chevron} {name}")
    } else {
        format!("{chevron} {name} ({count})")
    };
    let styled = theme.bold(&label);
    truncate_to_width(&styled, width)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ColorMode, SymbolPreset};
    use serde_json::json;
    use std::collections::HashMap;

    /// Minimal synthetic theme for deterministic tests.
    fn test_theme() -> Theme {
        let mut fg = HashMap::new();
        fg.insert("mdHeading".into(), json!("#ffcc00"));
        fg.insert("mdCode".into(), json!("#ff7b72"));
        fg.insert("mdCodeBlock".into(), json!("#c9d1d9"));
        fg.insert("mdCodeBlockBorder".into(), json!("#444"));
        fg.insert("error".into(), json!("#ff0000"));
        Theme::new(
            "test".into(),
            fg,
            HashMap::new(),
            ColorMode::Truecolor,
            SymbolPreset::Unicode,
            HashMap::new(),
            None,
            None,
        )
        .expect("test theme builds")
    }

    #[test]
    fn default_visibility_matches_dod() {
        let t = Transcript::new();
        assert_eq!(t.mode(Section::Thinking), SectionMode::Expanded);
        assert_eq!(t.mode(Section::Tools), SectionMode::Expanded);
        assert_eq!(t.mode(Section::Subagents), SectionMode::Collapsed);
        assert_eq!(t.mode(Section::Activity), SectionMode::Hidden);
        assert!(!t.all_hidden());
    }

    #[test]
    fn details_directive_switches_section() {
        let mut t = Transcript::new();
        assert!(t.details("activity expanded"));
        assert_eq!(t.mode(Section::Activity), SectionMode::Expanded);
        assert!(t.details("activity cycle"));
        assert_eq!(t.mode(Section::Activity), SectionMode::Hidden);
        // Unknown section or missing mode: no change.
        assert!(!t.details("bogus expanded"));
        assert!(!t.details("tools"));
        assert_eq!(t.mode(Section::Tools), SectionMode::Expanded);
    }

    #[test]
    fn all_hidden_triggers_backstop() {
        let mut t = Transcript::new();
        t.details("thinking hidden");
        t.details("tools hidden");
        t.details("subagents hidden");
        // activity already hidden by default.
        assert!(t.all_hidden());
        t.set_alert(Alert {
            text: "tool failed".into(),
        });
        let theme = test_theme();
        let rows = t.render(40, &theme);
        assert_eq!(rows.len(), 1, "only alert when all hidden: {rows:?}");
        assert!(rows[0].contains("tool failed"));
        // Without alert, render is empty.
        t.clear_alert();
        let rows = t.render(40, &theme);
        assert!(rows.is_empty());
    }

    #[test]
    fn collapsed_hides_body_expanded_shows_it() {
        let mut t = Transcript::new();
        t.push(Entry::new(Section::Thinking, "think step one"));
        t.push(Entry::new(Section::Thinking, "think step two"));

        let theme = test_theme();
        // Hide the other default-visible sections so only thinking renders.
        t.details("tools hidden");
        t.details("subagents hidden");

        // Collapsed: header only, with count, no body.
        t.details("thinking collapsed");
        let rows = t.render(40, &theme);
        assert_eq!(
            rows.len(),
            1,
            "only thinking header when collapsed: {rows:?}"
        );
        assert!(rows[0].contains("thinking (2)"));
        assert!(rows[0].contains('▸'));

        // Expanded: header + both bodies.
        t.details("thinking expanded");
        let rows = t.render(40, &theme);
        assert!(rows[0].contains('▾'));
        assert!(rows.iter().any(|r| r.contains("think step one")));
        assert!(rows.iter().any(|r| r.contains("think step two")));
    }

    #[test]
    fn hidden_section_renders_nothing() {
        let mut t = Transcript::new();
        t.push(Entry::new(Section::Activity, "ambient noise"));
        t.details("activity hidden");
        let theme = test_theme();
        let rows = t.render(40, &theme);
        assert!(!rows.iter().any(|r| r.contains("ambient noise")));
        assert!(!rows.iter().any(|r| r.contains("activity")));
    }

    #[test]
    fn multiple_sections_render_in_order() {
        let mut t = Transcript::new();
        t.push(Entry::new(Section::Thinking, "step 1"));
        t.push(Entry::new(Section::Tools, "tool result"));
        t.push(Entry::new(Section::Subagents, "agent report"));
        // activity is hidden by default.
        let theme = test_theme();
        let rows = t.render(40, &theme);
        // thinking header before tools header before subagents header.
        let idx_thinking = rows
            .iter()
            .position(|r| r.contains("thinking (1)"))
            .unwrap();
        let idx_tools = rows.iter().position(|r| r.contains("tools (1)")).unwrap();
        let idx_subagents = rows
            .iter()
            .position(|r| r.contains("subagents (1)"))
            .unwrap();
        assert!(idx_thinking < idx_tools);
        assert!(idx_tools < idx_subagents);
        // No activity header (hidden).
        assert!(!rows.iter().any(|r| r.contains("activity")));
    }

    #[test]
    fn global_details_mode_applies_to_every_section() {
        let mut t = Transcript::new();
        t.push(Entry::new(Section::Thinking, "think"));
        t.push(Entry::new(Section::Tools, "tool"));
        assert!(t.details("hidden"));
        assert!(t.all_hidden());
        // A lone section name is not a valid global mode.
        assert!(!t.details("tools"));
    }
}
