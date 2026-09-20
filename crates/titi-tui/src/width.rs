//! Width model: ANSI-aware UAX#11 width, truncation, column slicing, wrapping.
//! All render-path measurement/trimming must go through these helpers so that
//! escape sequences stay zero-width and column boundaries agree
//! (contract: `omp://tui-core-renderer`, spec: `docs/research/tui-renderer/width-model.md`).

use unicode_width::UnicodeWidthChar;

/// Tab advance used by the measurement model (flat, position-independent).
pub const DEFAULT_TAB_WIDTH: usize = 8;
pub const TAB: &str = "\t";

pub(crate) const RESET: &str = "\x1b[0m";

/// Token of a line: printable text or an escape sequence (zero-width, except OSC 66).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span<'a> {
    Text(&'a str),
    Escape(&'a str),
}

/// Iterator over [`Span`]s of `line` (single pass, no allocation).
#[derive(Debug)]
pub struct SpanIter<'a> {
    line: &'a str,
    pos: usize,
}

impl<'a> Iterator for SpanIter<'a> {
    type Item = Span<'a>;

    fn next(&mut self) -> Option<Span<'a>> {
        if self.pos >= self.line.len() {
            return None;
        }
        let rest = &self.line[self.pos..];
        if rest.starts_with('\x1b') {
            let len = escape_len(rest);
            self.pos += len;
            return Some(Span::Escape(&rest[..len]));
        }
        let len = rest.find('\x1b').unwrap_or(rest.len());
        self.pos += len;
        Some(Span::Text(&rest[..len]))
    }
}

/// One pass: text runs and escape sequences; escapes are never zero-width-split.
pub fn spans(line: &str) -> impl Iterator<Item = Span<'_>> {
    SpanIter { line, pos: 0 }
}

/// Byte length of the escape sequence at the start of `rest` (`rest` starts with ESC).
/// Truncated sequences consume the remainder (safe clamp, never panics).
fn escape_len(rest: &str) -> usize {
    let b = rest.as_bytes();
    if b.len() == 1 {
        return 1;
    }
    match b[1] {
        b'[' => {
            // CSI: parameter/intermediate bytes 0x20..=0x3F, final byte 0x40..=0x7E.
            let mut i = 2;
            while i < b.len() && (0x20..=0x3F).contains(&b[i]) {
                i += 1;
            }
            if i < b.len() && (0x40..=0x7E).contains(&b[i]) {
                i += 1;
            }
            i
        }
        b']' | b'P' | b'X' | b'^' | b'_' => {
            // String sequences (OSC/DCS/PM/APC): terminated by BEL or ST (ESC \).
            let mut i = 2;
            while i < b.len() {
                if b[i] == 0x07 {
                    return i + 1;
                }
                if b[i] == 0x1b && i + 1 < b.len() && b[i + 1] == b'\\' {
                    return i + 2;
                }
                i += 1;
            }
            b.len()
        }
        _ => {
            // Two-byte escape with optional intermediates (e.g. `ESC ( B`): ESC,
            // intermediates 0x20..=0x2F, one final byte. Non-ASCII is consumed
            // char-wise so slicing stays on char boundaries.
            let mut i = 1;
            for (off, c) in rest[1..].char_indices() {
                i = off + 1 + c.len_utf8();
                if !(c.is_ascii() && (0x20..=0x2F).contains(&(c as u8))) {
                    break;
                }
            }
            i
        }
    }
}

/// Width of an escape sequence: 0, except OSC 66 sized spans which announce
/// their width in cells (`OSC 66 ; cells ; payload`).
fn escape_width(seq: &str) -> usize {
    usize::from(osc66_cells(seq).unwrap_or(0))
}

/// Announced cell count of an OSC 66 sized span, if `seq` is one.
fn osc66_cells(seq: &str) -> Option<u16> {
    let inner = seq.strip_prefix("\x1b]")?;
    let payload = if let Some(stripped) = inner.strip_suffix('\x07') {
        stripped
    } else if let Some(stripped) = inner.strip_suffix("\x1b\\") {
        stripped
    } else {
        return None;
    };
    let mut fields = payload.split(';');
    if fields.next()? != "66" {
        return None;
    }
    fields.next()?.parse().ok()
}

fn is_sgr(seq: &str) -> bool {
    seq.starts_with("\x1b[") && seq.ends_with('m') && seq.len() > 2
}

/// True for a full SGR reset (`ESC[m`, `ESC[0m`, all-zero params).
fn is_sgr_reset(seq: &str) -> bool {
    if !is_sgr(seq) {
        return false;
    }
    let params = &seq[2..seq.len() - 1];
    params.is_empty() || params.split(';').all(|p| p == "0")
}

pub(crate) fn char_width(c: char) -> usize {
    if c == '\t' {
        DEFAULT_TAB_WIDTH
    } else {
        c.width().unwrap_or(0)
    }
}

/// Visible width in cells (UAX#11, ambiguous = narrow, CJK = 2, emoji = 2,
/// combining marks = 0); escape sequences are zero-width except OSC 66 spans.
pub fn visible_width(line: &str) -> usize {
    // Fast path: printable ASCII, no escapes, no tabs — one cell per byte.
    if line.bytes().all(|b| b.is_ascii_graphic() || b == b' ') {
        return line.len();
    }
    let mut w = 0;
    for span in spans(line) {
        match span {
            Span::Escape(seq) => w += escape_width(seq),
            Span::Text(t) => w += t.chars().map(char_width).sum::<usize>(),
        }
    }
    w
}

/// Truncate to at most `width` visible cells; a still-active SGR at the cut
/// point is closed with a reset. Clamps, never panics.
pub fn truncate_to_width(line: &str, width: usize) -> String {
    let mut out = String::new();
    let mut col = 0;
    let mut active_sgr = false;
    let mut truncated = false;
    'outer: for span in spans(line) {
        match span {
            Span::Escape(seq) => {
                out.push_str(seq);
                if is_sgr(seq) {
                    active_sgr = !is_sgr_reset(seq);
                }
            }
            Span::Text(t) => {
                for c in t.chars() {
                    let cw = char_width(c);
                    if col + cw > width {
                        truncated = true;
                        break 'outer;
                    }
                    col += cw;
                    out.push(c);
                }
            }
        }
    }
    if truncated && active_sgr {
        out.push_str(RESET);
    }
    out
}

/// Substring over columns `[start, end)` in cells; a char is included when it
/// starts within the range (wide chars straddling a boundary go to the range
/// they start in). SGR state seen before `start` is carried into the result,
/// and an SGR still active at the cut is closed so the slice renders
/// standalone. Ranges are expected to fall on character boundaries.
pub fn slice_by_column(line: &str, start: usize, end: usize) -> String {
    let mut prefix = String::new();
    let mut out = String::new();
    let mut col = 0;
    let mut active_sgr = false;
    'outer: for span in spans(line) {
        match span {
            Span::Escape(seq) => {
                if is_sgr(seq) {
                    active_sgr = !is_sgr_reset(seq);
                }
                if col < start {
                    prefix.push_str(seq);
                } else {
                    out.push_str(seq);
                }
            }
            Span::Text(t) => {
                for c in t.chars() {
                    if col >= end {
                        break 'outer;
                    }
                    let cw = char_width(c);
                    if col >= start {
                        out.push(c);
                    }
                    col += cw;
                }
            }
        }
    }
    if !out.is_empty() && active_sgr {
        out.push_str(RESET);
    }
    let mut result = prefix;
    result.push_str(&out);
    result
}

/// Wrap to `width` visible cells, word-wrapping on whitespace; words longer
/// than `width` are hard-clamped. SGR active at a break is closed on the
/// finished line and re-emitted at the start of the next one.
pub fn wrap_text_with_ansi(line: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut col = 0;
    let mut word = String::new();
    let mut word_w = 0;
    let mut sgr: Option<String> = None;

    fn break_line(lines: &mut Vec<String>, cur: &mut String, sgr: &Option<String>) {
        if sgr.is_some() {
            cur.push_str(RESET);
        }
        lines.push(std::mem::take(cur));
        if let Some(s) = sgr {
            cur.push_str(s);
        }
    }

    fn flush_word(
        word: &mut String,
        word_w: &mut usize,
        cur: &mut String,
        col: &mut usize,
        lines: &mut Vec<String>,
        sgr: &Option<String>,
        width: usize,
    ) {
        if *word_w > width {
            for c in word.chars() {
                let cw = char_width(c);
                if *col + cw > width {
                    break_line(lines, cur, sgr);
                    *col = 0;
                }
                cur.push(c);
                *col += cw;
            }
        } else {
            if *col + *word_w > width {
                break_line(lines, cur, sgr);
                *col = 0;
            }
            cur.push_str(word);
            *col += *word_w;
        }
        word.clear();
        *word_w = 0;
    }

    for span in spans(line) {
        match span {
            Span::Escape(seq) => {
                // An escape forces a word flush so it is emitted in place and
                // the SGR state at any later break is never ahead of the text.
                flush_word(
                    &mut word,
                    &mut word_w,
                    &mut cur,
                    &mut col,
                    &mut lines,
                    &sgr,
                    width,
                );
                if is_sgr(seq) {
                    if is_sgr_reset(seq) {
                        sgr = None;
                    } else {
                        sgr = Some(seq.to_owned());
                    }
                }
                cur.push_str(seq);
            }
            Span::Text(t) => {
                for c in t.chars() {
                    if c.is_whitespace() {
                        flush_word(
                            &mut word,
                            &mut word_w,
                            &mut cur,
                            &mut col,
                            &mut lines,
                            &sgr,
                            width,
                        );
                        // Whitespace is dropped at a line end, not carried over.
                        let ww = char_width(c);
                        if col + ww <= width {
                            cur.push(c);
                            col += ww;
                        }
                    } else {
                        word.push(c);
                        word_w += char_width(c);
                    }
                }
            }
        }
    }
    flush_word(
        &mut word,
        &mut word_w,
        &mut cur,
        &mut col,
        &mut lines,
        &sgr,
        width,
    );
    lines.push(cur);
    lines
}

/// Replace tabs with spaces up to `DEFAULT_TAB_WIDTH` tab stops (sanitization
/// of external input); escape sequences do not advance the column.
pub fn replace_tabs(line: &str) -> String {
    if !line.contains('\t') {
        return line.to_owned();
    }
    let mut out = String::with_capacity(line.len());
    let mut col = 0;
    for span in spans(line) {
        match span {
            Span::Escape(seq) => out.push_str(seq),
            Span::Text(t) => {
                for c in t.chars() {
                    if c == '\t' {
                        let pad = DEFAULT_TAB_WIDTH - col % DEFAULT_TAB_WIDTH;
                        out.extend(std::iter::repeat(' ').take(pad));
                        col += pad;
                    } else {
                        out.push(c);
                        col += char_width(c);
                    }
                }
            }
        }
    }
    out
}

/// Normalize ANSI at a line boundary: an SGR left active at end-of-line is
/// reset so an independently repainted line cannot inherit a neighbor's style.
pub fn normalize_ansi_boundary(line: &mut String) {
    let mut active = false;
    for span in spans(line) {
        if let Span::Escape(seq) = span {
            if is_sgr(seq) {
                active = !is_sgr_reset(seq);
            }
        }
    }
    if active {
        line.push_str(RESET);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_fast_path_counts_bytes() {
        assert_eq!(visible_width("hello"), 5);
        assert_eq!(visible_width(""), 0);
        assert_eq!(visible_width("a b!"), 4);
    }

    #[test]
    fn escapes_are_zero_width() {
        assert_eq!(visible_width("\x1b[31mred\x1b[0m"), 3);
        assert_eq!(visible_width("\x1b[2Jhi"), 2);
        assert_eq!(visible_width("\x1b[31"), 0); // truncated escape clamps to 0
        assert_eq!(visible_width("\x1b(Bx"), 1); // charset escape
        assert_eq!(visible_width("\x1b]0;title\x07ab"), 2); // OSC
    }

    #[test]
    fn cjk_is_two_cells() {
        assert_eq!(visible_width("中文"), 4);
        assert_eq!(visible_width("ab中cd"), 6);
    }

    #[test]
    fn emoji_is_two_cells() {
        assert_eq!(visible_width("👍"), 2);
        assert_eq!(visible_width("a😀b"), 4);
    }

    #[test]
    fn combining_marks_are_zero_width() {
        assert_eq!(visible_width("e\u{0301}"), 1);
    }

    #[test]
    fn ambiguous_is_narrow() {
        // U+00B1 (±) is East Asian Ambiguous; the narrow model gives it 1 cell.
        assert_eq!(visible_width("±"), 1);
    }

    #[test]
    fn osc66_span_announces_cells() {
        assert_eq!(visible_width("\x1b]66;10;x\x07"), 10);
        assert_eq!(visible_width("\x1b]66;3;y\x1b\\z"), 4);
        assert_eq!(visible_width("\x1b]65;10;x\x07"), 0); // not OSC 66
    }

    #[test]
    fn tab_is_default_tab_width() {
        assert_eq!(visible_width("\t"), DEFAULT_TAB_WIDTH);
    }

    #[test]
    fn truncate_closes_active_style() {
        assert_eq!(
            truncate_to_width("\x1b[31mhello world\x1b[0m", 5),
            "\x1b[31mhello\x1b[0m"
        );
        // Line already self-contained: untouched, no extra reset.
        assert_eq!(
            truncate_to_width("\x1b[31mhi\x1b[0m", 10),
            "\x1b[31mhi\x1b[0m"
        );
        assert_eq!(truncate_to_width("hello", 10), "hello");
    }

    #[test]
    fn truncate_clamps_wide_chars() {
        // 文 (2 cells) does not fit after 中; no later char is pulled up.
        assert_eq!(truncate_to_width("中文abc", 3), "中");
        assert_eq!(truncate_to_width("中文", 2), "中");
        assert_eq!(truncate_to_width("中文", 1), "");
        assert_eq!(truncate_to_width("hi", 0), "");
    }

    #[test]
    fn truncate_never_exceeds_width() {
        let samples = [
            "plain",
            "\x1b[31m中文 emoji 👍 tail\x1b[0m",
            "a\tb",
            "\x1b[7trunc\x1b",
        ];
        for s in samples {
            for w in 0..=12usize {
                assert!(
                    visible_width(&truncate_to_width(s, w)) <= w,
                    "s={s:?} w={w}"
                );
            }
        }
    }

    #[test]
    fn wrap_preserves_and_carries_style() {
        let wrapped = wrap_text_with_ansi("\x1b[31mhello world foo\x1b[0m", 5);
        assert_eq!(wrapped.len(), 3);
        assert_eq!(wrapped[0], "\x1b[31mhello\x1b[0m");
        assert_eq!(wrapped[1], "\x1b[31mworld\x1b[0m");
        assert_eq!(wrapped[2], "\x1b[31mfoo\x1b[0m");
        for line in &wrapped {
            assert!(visible_width(line) <= 5, "line={line:?}");
        }
    }

    #[test]
    fn wrap_clamps_long_words() {
        assert_eq!(wrap_text_with_ansi("abcdefgh", 3), vec!["abc", "def", "gh"]);
        assert_eq!(wrap_text_with_ansi("ab", 5), vec!["ab"]);
        assert_eq!(wrap_text_with_ansi("", 5), vec![""]);
    }

    #[test]
    fn wrap_emits_escape_in_place_and_carries_style() {
        // Escape ends the pending word; the style carries across the break.
        let wrapped = wrap_text_with_ansi("ab\x1b[1mcd", 3);
        assert_eq!(wrapped, vec!["ab\x1b[1m\x1b[0m", "\x1b[1mcd"]);
    }

    #[test]
    fn slice_by_column_cuts_cjk_like_terminal() {
        // "中文abc": 中=[0,2) 文=[2,4) a=[4) ...
        assert_eq!(slice_by_column("中文abc", 0, 4), "中文");
        assert_eq!(slice_by_column("中文abc", 2, 5), "文a");
        assert_eq!(slice_by_column("中文abc", 4, 6), "ab");
        // 中 starts at col 0, inside the range.
        assert_eq!(slice_by_column("中文abc", 0, 1), "中");
    }

    #[test]
    fn slice_carries_sgr_state() {
        let line = "\x1b[31mred中文\x1b[0m";
        let slice = slice_by_column(line, 3, 5);
        assert_eq!(slice, "\x1b[31m中\x1b[0m");
        assert_eq!(visible_width(&slice), 2);
    }

    #[test]
    fn replace_tabs_advances_to_tab_stops() {
        assert_eq!(replace_tabs("a\tb"), "a       b");
        assert_eq!(replace_tabs("\t"), "        ");
        assert_eq!(
            replace_tabs("\x1b[31m\tx\x1b[0m"),
            "\x1b[31m        x\x1b[0m"
        );
    }

    #[test]
    fn normalize_closes_trailing_sgr() {
        let mut line = String::from("ab\x1b[31m");
        normalize_ansi_boundary(&mut line);
        assert_eq!(line, "ab\x1b[31m\x1b[0m");

        let mut closed = String::from("ab\x1b[31m\x1b[0m");
        normalize_ansi_boundary(&mut closed);
        assert_eq!(closed, "ab\x1b[31m\x1b[0m");

        let mut plain = String::from("ab");
        normalize_ansi_boundary(&mut plain);
        assert_eq!(plain, "ab");
    }

    #[test]
    fn wrapped_lines_are_self_contained() {
        let line = "\x1b[32mprefix \x1b[1mbold long word here\x1b[0m tail";
        for w in [2usize, 5, 8, 20, 100] {
            for wrapped in wrap_text_with_ansi(line, w) {
                assert!(visible_width(&wrapped) <= w, "w={w} line={wrapped:?}");
                // Normalizing a wrapped line never needs to append a reset
                // (trailing reset was intentionally left open; normalization
                // closes it and keeps width unchanged since reset is 0-width).
                assert!(
                    visible_width(&wrapped)
                        == visible_width(&{
                            let mut l = wrapped.clone();
                            normalize_ansi_boundary(&mut l);
                            l
                        })
                );
            }
        }
    }
}
