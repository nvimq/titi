//! Transcript markdown renderer with theme tokens and section-visibility model.
//!
//! # Markdown supported
//!
//! - Headings (`#` … `######`) — styled with `ThemeColor::MdHeading`
//! - Bold (`**text**`) — with `Theme::bold`
//! - Italic (`*text*`) — with `Theme::italic`
//! - Inline code (`` `code` ``) — styled with `ThemeColor::MdCode`
//! - Fenced code blocks (```` ```lang ````) — styled with `ThemeColor::MdCodeBlock`
//! - Blockquotes (`> `) — styled with `ThemeColor::MdQuote` + `MdQuoteBorder`
//! - Unordered lists (`- `, `* `) — styled with `ThemeColor::MdListBullet`
//! - Ordered lists (`1. `) — numbered
//! - Horizontal rules (`---`) — styled with `ThemeColor::MdHr`
//! - Links (`[text](url)`) — text with `MdLink`, url with `MdLinkUrl`
//! - Paragraphs — wrapped to `width`
//!
//! # Section visibility
//!
//! The transcript is split into named sections: `thinking`, `tools`,
//! `subagents`, `activity`.  Each has a default mode per the DoD (thinking
//! and tools expanded, subagents collapsed, activity hidden).
//! `SectionVisibility::apply` implements `/details <section> <mode>`.

use crate::theme::{Theme, ThemeColor};
use crate::width::wrap_text_with_ansi;

// ---------------------------------------------------------------------------
// Section visibility
// ---------------------------------------------------------------------------

/// Per-section display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionMode {
    Hidden,
    Collapsed,
    Expanded,
}

/// Named transcript sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Thinking,
    Tools,
    Subagents,
    Activity,
}

impl Section {
    /// Parse a section name (`thinking`, `tools`, `subagents`, `activity`).
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "thinking" => Some(Section::Thinking),
            "tools" => Some(Section::Tools),
            "subagents" => Some(Section::Subagents),
            "activity" => Some(Section::Activity),
            _ => None,
        }
    }
}

/// Visibility state for each transcript section.
///
/// Defaults per DoD: thinking and tools expanded, subagents collapsed,
/// activity hidden.
#[derive(Debug, Clone)]
pub struct SectionVisibility {
    thinking: SectionMode,
    tools: SectionMode,
    subagents: SectionMode,
    activity: SectionMode,
}

impl Default for SectionVisibility {
    fn default() -> Self {
        SectionVisibility {
            thinking: SectionMode::Expanded,
            tools: SectionMode::Expanded,
            subagents: SectionMode::Collapsed,
            activity: SectionMode::Hidden,
        }
    }
}

impl SectionVisibility {
    /// Get the mode for a section.
    pub fn get(&self, section: Section) -> SectionMode {
        match section {
            Section::Thinking => self.thinking,
            Section::Tools => self.tools,
            Section::Subagents => self.subagents,
            Section::Activity => self.activity,
        }
    }

    /// Set the mode for a section.
    pub fn set(&mut self, section: Section, mode: SectionMode) {
        match section {
            Section::Thinking => self.thinking = mode,
            Section::Tools => self.tools = mode,
            Section::Subagents => self.subagents = mode,
            Section::Activity => self.activity = mode,
        }
    }

    /// Apply a `/details` directive.  Returns `true` if the state changed.
    ///
    /// Accepts `"hidden"`, `"collapsed"`, `"expanded"`, and `"cycle"` (next
    /// in the order: hidden → collapsed → expanded → hidden).
    pub fn apply(&mut self, section: Section, mode_str: &str) -> bool {
        let mode = match mode_str.trim().to_lowercase().as_str() {
            "hidden" => Some(SectionMode::Hidden),
            "collapsed" => Some(SectionMode::Collapsed),
            "expanded" => Some(SectionMode::Expanded),
            "cycle" => {
                let current = self.get(section);
                Some(match current {
                    SectionMode::Hidden => SectionMode::Collapsed,
                    SectionMode::Collapsed => SectionMode::Expanded,
                    SectionMode::Expanded => SectionMode::Hidden,
                })
            }
            _ => None,
        };
        match mode {
            Some(m) => {
                let old = self.get(section);
                self.set(section, m);
                old != m
            }
            None => false,
        }
    }

    /// Whether all sections are hidden — the app should show a floating alert.
    pub fn all_hidden(&self) -> bool {
        matches!(self.thinking, SectionMode::Hidden)
            && matches!(self.tools, SectionMode::Hidden)
            && matches!(self.subagents, SectionMode::Hidden)
            && matches!(self.activity, SectionMode::Hidden)
    }
}

// ---------------------------------------------------------------------------
// Markdown rendering
// ---------------------------------------------------------------------------

/// Render a markdown string to themed terminal lines.
///
/// `text` — raw markdown; `theme` — the active theme (provides token colours
/// and bold/italic helpers); `width` — column width to wrap paragraphs to.
pub fn render_markdown(text: &str, theme: &Theme, width: u16) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let w = width as usize;
    let mut lines = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_lines = Vec::new();

    for raw in text.lines() {
        if in_code_block {
            if raw.trim().starts_with("```") {
                // End of code block.
                lines.append(&mut render_code_block(&code_lines, &code_lang, theme, w));
                code_lines.clear();
                code_lang.clear();
                in_code_block = false;
                continue;
            }
            code_lines.push(raw);
            continue;
        }

        // Fenced code block start.
        if let Some(lang) = raw.trim().strip_prefix("```") {
            in_code_block = true;
            code_lang = lang.trim().to_owned();
            code_lines.clear();
            continue;
        }

        let trimmed = raw.trim();

        // Horizontal rule.
        if matches!(trimmed, "---" | "***" | "___") {
            lines.push(theme.fg(ThemeColor::MdHr, &"─".repeat(w.saturating_sub(1))));
            continue;
        }

        // Heading.
        if let Some(heading) = raw.strip_prefix('#') {
            let level = heading.chars().take_while(|c| *c == '#').count() + 1;
            let content = heading.trim_start_matches('#').trim();
            let styled = style_inline(content, theme);
            let prefix = "#".repeat(level);
            // Bold for headings.
            lines.push(theme.fg(ThemeColor::MdHeading, &format!("{prefix} {styled}")));
            continue;
        }

        // Blockquote.
        if let Some(content) = raw.strip_prefix('>') {
            let content = content.trim_start();
            let styled = style_inline(content, theme);
            let wrapped = wrap_text_with_ansi(&styled, w.saturating_sub(4));
            for (i, wline) in wrapped.iter().enumerate() {
                let border = if i == 0 { "▎ " } else { "  " };
                lines.push(format!(
                    "{}{}",
                    theme.fg(ThemeColor::MdQuoteBorder, border),
                    theme.fg(ThemeColor::MdQuote, wline)
                ));
            }
            continue;
        }

        // Unordered list.
        if let Some(rest) = raw
            .trim_start()
            .strip_prefix("- ")
            .or_else(|| raw.trim_start().strip_prefix("* "))
        {
            let content = style_inline(rest, theme);
            let bullet = theme.fg(ThemeColor::MdListBullet, "•");
            let wrapped = wrap_text_with_ansi(&content, w.saturating_sub(4));
            for (i, wline) in wrapped.iter().enumerate() {
                if i == 0 {
                    lines.push(format!("{bullet} {wline}"));
                } else {
                    lines.push(format!("  {wline}"));
                }
            }
            continue;
        }

        // Ordered list.
        if let Some(_) = raw
            .trim_start()
            .chars()
            .next()
            .filter(|c| c.is_ascii_digit())
        {
            if let Some(dot_pos) = raw.trim_start().find(". ") {
                let num_str = raw.trim_start()[..dot_pos].to_owned();
                let rest = raw.trim_start()[dot_pos + 2..].trim();
                let content = style_inline(rest, theme);
                let bullet = theme.fg(ThemeColor::MdListBullet, &format!("{num_str}."));
                let wrapped = wrap_text_with_ansi(&content, w.saturating_sub(4));
                for (i, wline) in wrapped.iter().enumerate() {
                    if i == 0 {
                        lines.push(format!("{bullet} {wline}"));
                    } else {
                        lines.push(format!("   {wline}"));
                    }
                }
                continue;
            }
        }

        // Empty line = paragraph break.
        if trimmed.is_empty() {
            lines.push(String::new());
            continue;
        }

        // Plain paragraph.
        let styled = style_inline(trimmed, theme);
        lines.append(&mut wrap_text_with_ansi(&styled, w));
    }

    // Flush any trailing code block.
    if in_code_block && !code_lines.is_empty() {
        lines.append(&mut render_code_block(&code_lines, &code_lang, theme, w));
    }

    lines
}

/// Render a fenced code block.
fn render_code_block(lines: &[&str], lang: &str, theme: &Theme, w: usize) -> Vec<String> {
    let mut out = Vec::new();
    if !lang.is_empty() {
        out.push(theme.fg(ThemeColor::MdCodeBlock, &format!("```{lang}")));
    }
    for line in lines {
        out.push(theme.fg(ThemeColor::MdCodeBlock, line));
    }
    out.push(theme.fg(
        ThemeColor::MdCodeBlockBorder,
        &"─".repeat(w.saturating_sub(1)),
    ));
    out
}

/// Style inline markdown in a single line of text.
///
/// Handles `` `code` ``, `[text](url)`, `**bold**`, `*italic*`.  Processes
/// the earliest marker first and recurses into prefixes so nested/staged
/// markers (e.g. bold before an inline code span) all render.
fn style_inline(text: &str, theme: &Theme) -> String {
    let mut out = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        let code_at = rest.find('`');
        let link_at = rest.find('[');
        let bold_at = rest.find("**");
        let italic_at = rest.find('*');

        // Earliest marker wins; on ties code > link > bold > italic.
        let mut best: Option<(usize, &str)> = None;
        for (i, kind) in [
            (code_at, "code"),
            (link_at, "link"),
            (bold_at, "bold"),
            (italic_at, "italic"),
        ] {
            let Some(i) = i else { continue };
            if kind == "italic" && bold_at == Some(i) {
                continue; // part of a bold pair
            }
            if best.is_none_or(|(b, _)| i < b) {
                best = Some((i, kind));
            }
        }

        let Some((i, kind)) = best else {
            out.push_str(rest);
            break;
        };

        // Recurse into the plain prefix so markers before this one render.
        out.push_str(&style_inline(&rest[..i], theme));
        rest = &rest[i..];

        match kind {
            "code" => {
                rest = &rest[1..];
                if let Some(end) = rest.find('`') {
                    out.push_str(&theme.fg(ThemeColor::MdCode, &rest[..end]));
                    rest = &rest[end + 1..];
                } else {
                    out.push('`');
                }
            }
            "link" => {
                rest = &rest[1..];
                if let Some(end) = rest.find(']') {
                    let link_text = &rest[..end];
                    rest = &rest[end + 1..];
                    if rest.starts_with('(') {
                        rest = &rest[1..];
                        if let Some(url_end) = rest.find(')') {
                            let url = &rest[..url_end];
                            out.push_str(&theme.fg(ThemeColor::MdLink, link_text));
                            out.push_str(&theme.fg(ThemeColor::MdLinkUrl, &format!(" ({url})")));
                            rest = &rest[url_end + 1..];
                            continue;
                        }
                    }
                    // No matching URL — literal.
                    out.push('[');
                    out.push_str(link_text);
                    out.push(']');
                } else {
                    out.push('[');
                }
            }
            "bold" => {
                rest = &rest[2..];
                if let Some(end) = rest.find("**") {
                    let inner = style_inline(&rest[..end], theme);
                    out.push_str(&theme.bold(&inner));
                    rest = &rest[end + 2..];
                } else {
                    out.push_str("**");
                }
            }
            "italic" => {
                rest = &rest[1..];
                if let Some(end) = rest.find('*') {
                    // Do not treat a `**` closing as a lone italic marker.
                    let inner = style_inline(&rest[..end], theme);
                    out.push_str(&theme.italic(&inner));
                    rest = &rest[end + 1..];
                } else {
                    out.push('*');
                }
            }
            _ => unreachable!(),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use std::collections::HashMap;

    fn test_theme() -> Theme {
        Theme::new(
            "test".into(),
            HashMap::new(),
            HashMap::new(),
            crate::theme::ColorMode::Color256,
            crate::theme::SymbolPreset::Unicode,
            HashMap::new(),
            None,
            None,
        )
        .expect("theme builds")
    }

    // ---- SectionVisibility ------------------------------------------------

    #[test]
    fn section_defaults() {
        let v = SectionVisibility::default();
        assert_eq!(v.get(Section::Thinking), SectionMode::Expanded);
        assert_eq!(v.get(Section::Tools), SectionMode::Expanded);
        assert_eq!(v.get(Section::Subagents), SectionMode::Collapsed);
        assert_eq!(v.get(Section::Activity), SectionMode::Hidden);
        assert!(!v.all_hidden());
    }

    #[test]
    fn section_apply_hidden() {
        let mut v = SectionVisibility::default();
        assert!(v.apply(Section::Thinking, "hidden"));
        assert_eq!(v.get(Section::Thinking), SectionMode::Hidden);
    }

    #[test]
    fn section_apply_cycle_through() {
        let mut v = SectionVisibility::default();
        assert_eq!(v.get(Section::Thinking), SectionMode::Expanded);
        assert!(v.apply(Section::Thinking, "cycle"));
        assert_eq!(v.get(Section::Thinking), SectionMode::Hidden);
        assert!(v.apply(Section::Thinking, "cycle"));
        assert_eq!(v.get(Section::Thinking), SectionMode::Collapsed);
        assert!(v.apply(Section::Thinking, "cycle"));
        assert_eq!(v.get(Section::Thinking), SectionMode::Expanded);
    }

    #[test]
    fn section_apply_invalid_noop() {
        let mut v = SectionVisibility::default();
        assert!(!v.apply(Section::Thinking, "bogus"));
        assert_eq!(v.get(Section::Thinking), SectionMode::Expanded);
    }

    #[test]
    fn section_invalid_name() {
        assert!(Section::parse("bogus").is_none());
        assert_eq!(Section::parse("thinking"), Some(Section::Thinking));
        assert_eq!(Section::parse("tools"), Some(Section::Tools));
        assert_eq!(Section::parse("subagents"), Some(Section::Subagents));
        assert_eq!(Section::parse("activity"), Some(Section::Activity));
    }

    #[test]
    fn all_hidden_true_when_everything_hidden() {
        let mut v = SectionVisibility::default();
        v.apply(Section::Thinking, "hidden");
        v.apply(Section::Tools, "hidden");
        v.apply(Section::Subagents, "hidden");
        v.apply(Section::Activity, "hidden");
        assert!(v.all_hidden());
    }

    // ---- Markdown rendering -----------------------------------------------

    #[test]
    fn empty_text() {
        let lines = render_markdown("", &test_theme(), 80);
        assert!(lines.is_empty());
    }

    #[test]
    fn heading_rendered() {
        let theme = test_theme();
        let lines = render_markdown("# Hello", &theme, 80);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("# Hello"));
    }

    #[test]
    fn bold_rendered() {
        let theme = test_theme();
        let lines = render_markdown("this is **bold** text", &theme, 80);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].contains("\x1b[1m"),
            "bold should use ANSI bold: {lines:?}"
        );
        theme.bold("bold");
        // The bold ANSI escape should be present.
        assert!(lines[0].contains("bold"), "bold word should appear");
    }

    #[test]
    fn italic_rendered() {
        let theme = test_theme();
        let lines = render_markdown("this is *italic* text", &theme, 80);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].contains("\x1b[3m"),
            "italic should use ANSI italic: {lines:?}"
        );
    }

    #[test]
    fn inline_code_rendered() {
        let theme = test_theme();
        let lines = render_markdown("use `ffmpeg` to convert", &theme, 80);
        // mdCode token not in test theme (empty map), so returns to default.
        // The word `ffmpeg` should be present.
        assert!(
            lines[0].contains("ffmpeg"),
            "code word should appear: {lines:?}"
        );
    }

    #[test]
    fn link_rendered() {
        let theme = test_theme();
        let lines = render_markdown("click [here](https://example.com)", &theme, 80);
        assert!(
            lines[0].contains("here"),
            "link text should appear: {lines:?}"
        );
        assert!(
            lines[0].contains("example.com"),
            "url should appear: {lines:?}"
        );
    }

    #[test]
    fn unordered_list_rendered() {
        let theme = test_theme();
        let lines = render_markdown("- item one\n- item two", &theme, 80);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("item one"), "first item: {lines:?}");
        assert!(lines[1].contains("item two"), "second item: {lines:?}");
    }

    #[test]
    fn ordered_list_rendered() {
        let theme = test_theme();
        let lines = render_markdown("1. first\n2. second", &theme, 80);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("first"), "first item: {lines:?}");
        assert!(lines[1].contains("second"), "second item: {lines:?}");
    }

    #[test]
    fn blockquote_rendered() {
        let theme = test_theme();
        let lines = render_markdown("> quoted text", &theme, 80);
        assert!(lines[0].contains("quoted text"), "blockquote: {lines:?}");
        // The border character should be present.
        assert!(lines[0].contains("▎"), "blockquote border: {lines:?}");
    }

    #[test]
    fn code_block_rendered() {
        let theme = test_theme();
        let lines = render_markdown("```rust\nfn main() {}\n```", &theme, 80);
        assert!(
            lines.iter().any(|l| l.contains("fn main()")),
            "code block: {lines:?}"
        );
    }

    #[test]
    fn horizontal_rule_rendered() {
        let theme = test_theme();
        let lines = render_markdown("---", &theme, 80);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains('─'), "hr: {lines:?}");
    }

    #[test]
    fn paragraph_wrapping() {
        let theme = test_theme();
        let long = "This is a very long paragraph that should wrap at the given width limit.";
        let lines = render_markdown(long, &theme, 20);
        // At width 20, should produce at least 2 lines.
        assert!(lines.len() >= 2, "should wrap: {lines:?}");
    }

    #[test]
    fn multiple_heading_levels() {
        let theme = test_theme();
        let lines = render_markdown("## Subheading\n### Subsub", &theme, 80);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("## Subheading"));
        assert!(lines[1].contains("### Subsub"));
    }

    #[test]
    fn bold_and_italic_together() {
        let theme = test_theme();
        let lines = render_markdown("**bold** and *italic*", &theme, 80);
        assert!(lines[0].contains("\x1b[1m"), "bold marker");
        assert!(lines[0].contains("\x1b[3m"), "italic marker");
    }
}
