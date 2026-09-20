//! Viewport diffing: emit only the rows of the next frame that differ from
//! the previous one (contract: `omp://tui-core-renderer`,
//! spec: `docs/research/tui-renderer/frame-history.md`).

/// A single viewport row to repaint. `row` is the full replacement line
/// (ANSI included); an empty `row` clears the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowUpdate {
    pub index: usize,
    pub row: String,
}

/// Diff `prev` against `next`: updates for changed rows, appended rows, and —
/// for a truncated screen (next shorter than prev) — empty-row clears for
/// every index beyond `next.len()` so no stale row lingers. Comparison is
/// byte-for-byte; width stabilization happens in the width module upstream.
pub fn diff_viewport(prev: &[String], next: &[String]) -> Vec<RowUpdate> {
    let mut updates: Vec<RowUpdate> = next
        .iter()
        .enumerate()
        .filter(|(i, row)| prev.get(*i) != Some(*row))
        .map(|(index, row)| RowUpdate {
            index,
            row: row.clone(),
        })
        .collect();
    if next.len() < prev.len() {
        updates.extend((next.len()..prev.len()).map(|index| RowUpdate {
            index,
            row: String::new(),
        }));
    }
    updates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows<const N: usize>(vals: [&str; N]) -> Vec<String> {
        vals.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn identical_viewports_diff_to_nothing() {
        let v = rows(["a", "b", "c"]);
        assert!(diff_viewport(&v, &v).is_empty());
    }

    #[test]
    fn changed_rows_are_reported_with_indices() {
        let prev = rows(["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]);
        let mut next = prev.clone();
        next[3] = "changed".into();
        next[9] = "also".into();
        let updates = diff_viewport(&prev, &next);
        assert_eq!(updates.len(), 2);
        assert_eq!(
            updates[0],
            RowUpdate {
                index: 3,
                row: "changed".into()
            }
        );
        assert_eq!(
            updates[1],
            RowUpdate {
                index: 9,
                row: "also".into()
            }
        );
    }

    #[test]
    fn appended_rows_are_reported() {
        let prev = rows(["a", "b"]);
        let next = rows(["a", "b", "c", "d"]);
        let updates = diff_viewport(&prev, &next);
        assert_eq!(
            updates,
            vec![
                RowUpdate {
                    index: 2,
                    row: "c".into()
                },
                RowUpdate {
                    index: 3,
                    row: "d".into()
                },
            ]
        );
    }

    #[test]
    fn truncated_screen_clears_stale_rows() {
        let prev = rows(["a", "b", "c", "d", "e"]);
        let next = rows(["a", "b"]);
        let updates = diff_viewport(&prev, &next);
        // Unchanged prefix: nothing; stale tail: empty-row clears.
        assert_eq!(
            updates,
            vec![
                RowUpdate {
                    index: 2,
                    row: String::new()
                },
                RowUpdate {
                    index: 3,
                    row: String::new()
                },
                RowUpdate {
                    index: 4,
                    row: String::new()
                },
            ]
        );
    }

    #[test]
    fn truncated_and_rewritten_screen_reports_both() {
        let prev = rows(["a", "b", "c", "d"]);
        let next = rows(["x", "b"]);
        let updates = diff_viewport(&prev, &next);
        assert_eq!(
            updates,
            vec![
                RowUpdate {
                    index: 0,
                    row: "x".into()
                },
                RowUpdate {
                    index: 2,
                    row: String::new()
                },
                RowUpdate {
                    index: 3,
                    row: String::new()
                },
            ]
        );
    }

    #[test]
    fn empty_next_clears_everything() {
        let prev = rows(["a", "b"]);
        assert!(diff_viewport(&prev, &[]).is_empty() == false);
        let updates = diff_viewport(&prev, &[]);
        assert_eq!(updates.len(), 2);
        assert!(updates.iter().all(|u| u.row.is_empty()));
    }
}
