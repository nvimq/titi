//! Cursor transport: components embed a private-use marker in rendered text;
//! [`extract_cursor`] pulls the position out and strips the marker so it
//! never leaks into terminal output
//! (contract: `omp://tui`, spec: `docs/research/tui-renderer/README.md` §3).

use crate::width::{self, Span};

/// Private-use character (U+E000) marking the cursor position in a row.
pub const CURSOR_MARKER: char = '\u{E000}';

/// Extract the cursor position from rendered rows, stripping every marker
/// occurrence from `rows` in place.
///
/// Returns the first marker as `(row_index, column)`, where `column` is the
/// visible cell offset of the marker inside its row (ANSI escapes are
/// zero-width, wide chars count as 2). Returns `None` when no row carries a
/// marker.
pub fn extract_cursor(rows: &mut [String]) -> Option<(usize, usize)> {
    let mut cursor: Option<(usize, usize)> = None;
    for (row_index, row) in rows.iter_mut().enumerate() {
        if !row.contains(CURSOR_MARKER) {
            continue;
        }
        let mut column = 0usize;
        let mut row_cursor: Option<usize> = None;
        let mut cleaned = String::with_capacity(row.len());
        for span in width::spans(row) {
            match span {
                Span::Escape(seq) => cleaned.push_str(seq),
                Span::Text(text) => {
                    for c in text.chars() {
                        if c == CURSOR_MARKER {
                            // First marker in the first marked row wins;
                            // all markers are dropped from the output.
                            if row_cursor.is_none() {
                                row_cursor = Some(column);
                            }
                        } else {
                            cleaned.push(c);
                            column += width::char_width(c);
                        }
                    }
                }
            }
        }
        *row = cleaned;
        if let Some(col) = row_cursor {
            if cursor.is_none() {
                cursor = Some((row_index, col));
            }
        }
    }
    cursor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_marker_returns_none_and_keeps_rows() {
        let mut rows = vec!["plain".to_owned(), "\x1b[31mred\x1b[0m".to_owned()];
        let before = rows.clone();
        assert_eq!(extract_cursor(&mut rows), None);
        assert_eq!(rows, before);
    }

    #[test]
    fn marker_position_is_row_and_cell_column() {
        let mut rows = vec!["ab".to_owned(), format!("cd{}ef", CURSOR_MARKER)];
        assert_eq!(extract_cursor(&mut rows), Some((1, 2)));
        assert_eq!(rows[1], "cdef");
    }

    #[test]
    fn marker_is_removed_from_output() {
        let mut rows = vec![format!("x{}y", CURSOR_MARKER)];
        assert_eq!(extract_cursor(&mut rows), Some((0, 1)));
        assert_eq!(rows, vec!["xy".to_owned()]);
        // A second pass finds nothing — the marker is gone for good.
        assert_eq!(extract_cursor(&mut rows), None);
    }

    #[test]
    fn ansi_escapes_do_not_advance_the_column() {
        let mut rows = vec![format!("\x1b[1mab\x1b[0m{}c", CURSOR_MARKER)];
        assert_eq!(extract_cursor(&mut rows), Some((0, 2)));
        assert_eq!(rows[0], "\x1b[1mab\x1b[0mc");
    }

    #[test]
    fn wide_chars_count_double() {
        let mut rows = vec![format!("日{}本", CURSOR_MARKER)];
        assert_eq!(extract_cursor(&mut rows), Some((0, 2)));
        assert_eq!(rows[0], "日本");
    }

    #[test]
    fn marker_at_start_of_row_and_of_frame() {
        let mut rows = vec![CURSOR_MARKER.to_string(), "rest".to_owned()];
        assert_eq!(extract_cursor(&mut rows), Some((0, 0)));
        assert_eq!(rows[0], "");
    }

    #[test]
    fn first_marker_wins_and_all_markers_are_stripped() {
        let mut rows = vec![
            format!("a{}b{}", CURSOR_MARKER, CURSOR_MARKER),
            format!("{}z", CURSOR_MARKER),
        ];
        assert_eq!(extract_cursor(&mut rows), Some((0, 1)));
        assert_eq!(rows, vec!["ab".to_owned(), "z".to_owned()]);
    }

    #[test]
    fn empty_rows_are_fine() {
        let mut rows: Vec<String> = Vec::new();
        assert_eq!(extract_cursor(&mut rows), None);
    }
}
