//! Frame renderer — history batch + ack handshake, viewport diffing,
//! overlay compositing, synchronized output, cursor park, and resize
//! policies (`Preserve` / `Append` / `Rebuild`).
//!
//! Contract: `docs/research/tui-renderer/frame-history.md`.

use std::io::{self, Write};

use crate::caps;
use crate::history::{accept_batch, Ack, HistoryBatch, HistoryState};
use crate::overlay::OverlayStack;
use crate::viewport::diff_viewport;

/// A frame the provider produces: optional history batch plus the viewport.
#[derive(Debug, Clone)]
pub struct FramePlan {
    /// History batch to commit (append or replay); `None` = viewport-only
    /// frame that never touches scrollback.
    pub history: Option<HistoryBatch>,
    /// Viewport rows for the current frame.
    pub viewport: Vec<String>,
}

/// The frame provider — the application side of the batch/ack handshake.
///
/// `plan()` returns the next frame; `acknowledge()` is called by the
/// renderer when the batch has been written to the terminal.
pub trait FrameProvider {
    /// Produce the next frame for the given terminal size.
    fn plan(&mut self, size: (u16, u16)) -> FramePlan;
    /// Confirm that the batch with `id` has been committed to the terminal.
    fn acknowledge(&mut self, id: u64);
}

/// Resize policy: what happens to scrollback history when the terminal
/// is resized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResizeScrollbackMode {
    /// Only the viewport is repainted; history is untouched.
    #[default]
    Preserve,
    /// Archived history (reflowed) is written below the existing scrollback
    /// as a replay batch.
    Append,
    /// Clear terminal scrollback (`ED3`) and replay the entire transcript
    /// at the new width.
    Rebuild,
}

/// The terminal renderer.
///
/// Holds the output writer, accumulated history state, overlay stack, and
/// resize policy.  One `draw()` call = one atomic frame.
pub struct Renderer<W: Write> {
    out: W,
    /// Accumulated history (last-acked id + rows).
    history_state: HistoryState,
    /// Number of history rows already written to the terminal.
    history_written: usize,
    /// Previous viewport for diffing.
    prev_viewport: Vec<String>,
    /// Overlay stack.
    overlays: OverlayStack,
    /// Whether to wrap frame output in synchronized output markers.
    sync_output: bool,
    /// Resize policy.
    resize_mode: ResizeScrollbackMode,
    /// Terminal width (cells).
    width: u16,
    /// Terminal height (cells).
    height: u16,
}

impl<W: Write> Renderer<W> {
    /// Create a new renderer.
    pub fn new(
        out: W,
        width: u16,
        height: u16,
        sync_output: bool,
        resize_mode: ResizeScrollbackMode,
    ) -> Self {
        Renderer {
            out,
            history_state: HistoryState::new(),
            history_written: 0,
            prev_viewport: Vec::new(),
            overlays: OverlayStack::new(),
            sync_output,
            resize_mode,
            width,
            height,
        }
    }

    // ------------------------------------------------------------------
    // Public API
    // ------------------------------------------------------------------

    /// Draw one frame from `plan`.
    ///
    /// 1. Accept and write the history batch (if any).
    /// 2. Composite overlays on top of the viewport.
    /// 3. Diff against the previous viewport and write only changed rows.
    /// 4. Park the cursor at the bottom of the viewport.
    /// 5. Flush.
    ///
    /// Returns the [`Ack`] if a history batch was accepted, or `None` for
    /// viewport-only frames / rejected batches.
    pub fn draw(&mut self, plan: FramePlan) -> io::Result<Option<Ack>> {
        let ack = self.write_history(plan.history)?;

        // Composite overlays onto the viewport.
        let viewport = self.overlays.composite(&plan.viewport, self.width);
        let updates = diff_viewport(&self.prev_viewport, &viewport);

        // Write viewport updates, wrapped in sync output markers.
        if !updates.is_empty() {
            if self.sync_output {
                write!(self.out, "{}", caps::sync_begin())?;
            }

            for update in &updates {
                let row = update.index + 1; // 1-based terminal row
                if update.row.is_empty() {
                    // Clear the line.
                    write!(self.out, "\x1b[{}H\x1b[2K", row)?;
                } else {
                    write!(self.out, "\x1b[{}H{}", row, update.row)?;
                }
            }

            // Park cursor at the bottom of the viewport (row after the last
            // viewport row, i.e. the status line).
            let park_row = viewport.len() + 1;
            write!(self.out, "\x1b[{}H", park_row)?;

            if self.sync_output {
                write!(self.out, "{}", caps::sync_end())?;
            }
        }

        self.out.flush()?;
        self.prev_viewport = viewport;

        Ok(ack)
    }

    /// Handle terminal resize: apply the configured resize policy, then
    /// re-plan and redraw.
    pub fn on_resize(
        &mut self,
        new_width: u16,
        new_height: u16,
        provider: &mut dyn FrameProvider,
    ) -> io::Result<Option<Ack>> {
        self.width = new_width;
        self.height = new_height;

        match self.resize_mode {
            ResizeScrollbackMode::Preserve => {
                // Only viewport repaint — no history writes.
                self.prev_viewport.clear();
            }
            ResizeScrollbackMode::Append => {
                // Archive existing history: everything must be replayed
                // below the existing scrollback.
                self.history_written = 0;
                self.prev_viewport.clear();
            }
            ResizeScrollbackMode::Rebuild => {
                // Clear native terminal scrollback (ED3).
                write!(self.out, "\x1b[3J")?;
                self.out.flush()?;
                // Reset state: everything must be replayed.
                self.history_state = HistoryState::new();
                self.history_written = 0;
                self.prev_viewport.clear();
            }
        }

        // Get a fresh plan from the provider at the new size, write it,
        // then ack strictly after the write.
        let plan = provider.plan((new_width, new_height));
        let ack = self.draw(plan)?;
        if let Some(ack) = ack {
            provider.acknowledge(ack.id);
        }
        Ok(ack)
    }

    /// Destructive display reset: ED3 + re-offer the full acked history
    /// under a new monotonic id (only for user gestures: session replacement,
    /// Ctrl+L, etc.).
    pub fn reset_display(
        &mut self,
        provider: &mut dyn FrameProvider,
    ) -> io::Result<Option<Ack>> {
        // Clear native scrollback.
        write!(self.out, "\x1b[3J")?;
        self.out.flush()?;

        // The provider must re-offer the full history under new ids.
        self.history_state = HistoryState::new();
        self.history_written = 0;
        self.prev_viewport.clear();

        let plan = provider.plan((self.width, self.height));
        let ack = self.draw(plan)?;
        if let Some(ack) = ack {
            provider.acknowledge(ack.id);
        }
        Ok(ack)
    }

    // ------------------------------------------------------------------
    // Overlay helpers
    // ------------------------------------------------------------------

    /// Show an overlay on the next frame.
    pub fn show_overlay(
        &mut self,
        component: Box<dyn crate::component::Component>,
        anchor: crate::overlay::Anchor,
    ) -> crate::overlay::OverlayHandle {
        self.overlays.show(component, anchor)
    }

    /// Hide an overlay.
    pub fn hide_overlay(
        &mut self,
        handle: crate::overlay::OverlayHandle,
    ) -> Option<Box<dyn crate::component::Component>> {
        self.overlays.hide(handle)
    }

    /// Reference to the overlay stack.
    pub fn overlays(&self) -> &OverlayStack {
        &self.overlays
    }

    /// Mutable reference to the overlay stack.
    pub fn overlays_mut(&mut self) -> &mut OverlayStack {
        &mut self.overlays
    }

    // ------------------------------------------------------------------
    // Accessors
    // ------------------------------------------------------------------

    /// Current terminal width.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Current terminal height.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Resize mode.
    pub fn resize_mode(&self) -> ResizeScrollbackMode {
        self.resize_mode
    }

    /// Set resize mode.
    pub fn set_resize_mode(&mut self, mode: ResizeScrollbackMode) {
        self.resize_mode = mode;
    }

    /// Whether sync output is enabled.
    pub fn sync_output(&self) -> bool {
        self.sync_output
    }

    /// Enable/disable sync output wrapping.
    pub fn set_sync_output(&mut self, enabled: bool) {
        self.sync_output = enabled;
    }

    /// Reference to the underlying writer.
    pub fn out(&self) -> &W {
        &self.out
    }

    /// Mutable reference to the underlying writer.
    pub fn out_mut(&mut self) -> &mut W {
        &mut self.out
    }

    /// Consume the renderer and return the underlying writer.
    pub fn into_inner(self) -> W {
        self.out
    }

    // ------------------------------------------------------------------
    // Internal
    // ------------------------------------------------------------------

    /// Accept and write the history batch (if any).
    fn write_history(&mut self, batch: Option<HistoryBatch>) -> io::Result<Option<Ack>> {
        let batch = match batch {
            Some(b) => b,
            None => return Ok(None),
        };

        match accept_batch(&mut self.history_state, batch) {
            Ok(ack) => {
                // Write all history rows (replay) or only the newly appended
                // tail (append).
                let rows = self.history_state.rows();
                for row in &rows[self.history_written..] {
                    writeln!(self.out, "{row}")?;
                }
                self.history_written = rows.len();
                self.out.flush()?;
                Ok(Some(ack))
            }
            Err(_) => {
                // Duplicate / non-monotonic — silently skip.
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::BatchKind;
    use crate::overlay::Anchor;

    /// A trivial writer that captures all output.
    #[derive(Default)]
    struct TestWriter {
        data: Vec<u8>,
    }

    impl Write for TestWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.data.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl TestWriter {
        fn output(&self) -> String {
            String::from_utf8_lossy(&self.data).to_string()
        }
    }

    /// Local mock component (component::Mock is private to that module).
    struct Mock {
        content: Vec<String>,
    }

    impl Mock {
        fn new(rows: &[&str]) -> Self {
            Mock {
                content: rows.iter().map(|s| (*s).to_owned()).collect(),
            }
        }
    }

    impl crate::component::Component for Mock {
        fn render(&mut self, _width: u16) -> Vec<String> {
            self.content.clone()
        }

        fn handle_input(&mut self, _data: &str) {}
    }

    /// A simple provider that returns canned plans.
    struct TestProvider {
        plan: FramePlan,
        acked: Vec<u64>,
    }

    impl TestProvider {
        fn new(plan: FramePlan) -> Self {
            TestProvider {
                plan,
                acked: Vec::new(),
            }
        }
    }

    impl FrameProvider for TestProvider {
        fn plan(&mut self, _size: (u16, u16)) -> FramePlan {
            self.plan.clone()
        }
        fn acknowledge(&mut self, id: u64) {
            self.acked.push(id);
        }
    }

    fn hb(id: u64, rows: &[&str], kind: BatchKind) -> HistoryBatch {
        HistoryBatch {
            id,
            rows: rows.iter().map(|s| (*s).to_owned()).collect(),
            kind,
        }
    }

    fn renderer(
        w: u16,
        h: u16,
        sync: bool,
        mode: ResizeScrollbackMode,
    ) -> Renderer<TestWriter> {
        Renderer::new(TestWriter::default(), w, h, sync, mode)
    }

    // ---- Handshake --------------------------------------------------------

    #[test]
    fn handshake_accepts_monotonic_batches() {
        let mut r = renderer(80, 24, false, ResizeScrollbackMode::Preserve);
        let plan1 = FramePlan {
            history: Some(hb(1, &["a"], BatchKind::Append)),
            viewport: vec!["b".into()],
        };
        let ack = r.draw(plan1).unwrap();
        assert_eq!(ack, Some(Ack { id: 1 }), "first batch acked");

        let plan2 = FramePlan {
            history: Some(hb(2, &["c"], BatchKind::Append)),
            viewport: vec![],
        };
        let ack = r.draw(plan2).unwrap();
        assert_eq!(ack, Some(Ack { id: 2 }), "second batch acked");
    }

    #[test]
    fn duplicate_batch_is_not_written_twice() {
        let mut r = renderer(80, 24, false, ResizeScrollbackMode::Preserve);
        let plan = FramePlan {
            history: Some(hb(1, &["a"], BatchKind::Append)),
            viewport: vec![],
        };
        let ack1 = r.draw(plan.clone()).unwrap();
        assert_eq!(ack1, Some(Ack { id: 1 }));

        // Same id offered again (provider retry before ack): not written twice.
        let ack2 = r.draw(plan).unwrap();
        assert_eq!(ack2, None, "duplicate batch rejected");

        let output = r.out().output();
        assert_eq!(output.matches("a").count(), 1, "row written exactly once");
    }

    #[test]
    fn viewport_only_frame_writes_no_history() {
        let mut r = renderer(80, 24, false, ResizeScrollbackMode::Preserve);
        let plan = FramePlan {
            history: None,
            viewport: vec!["hello".into()],
        };
        let ack = r.draw(plan).unwrap();
        assert_eq!(ack, None, "viewport-only frame emits no ack");

        // No history rows in output.
        let output = r.out().output();
        assert!(!output.contains("hello\n"), "no history write");
    }

    // ---- Viewport diffing -------------------------------------------------

    #[test]
    fn diff_emits_only_changed_rows() {
        let mut r = renderer(80, 24, false, ResizeScrollbackMode::Preserve);

        // First frame: full viewport.
        let rows: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        r.draw(FramePlan { history: None, viewport: rows }).unwrap();

        // Second frame: rows 3 and 9 changed.
        let mut next: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        next[3] = "changed3".into();
        next[9] = "changed9".into();
        r.out_mut().data.clear();
        r.draw(FramePlan { history: None, viewport: next }).unwrap();

        let diff = r.out().output();
        assert!(diff.contains("\x1b[4Hchanged3"), "diff writes row 3");
        assert!(diff.contains("\x1b[10Hchanged9"), "diff writes row 9");
        assert!(!diff.contains("\x1b[1H0"), "unchanged row 0 not rewritten");
    }

    #[test]
    fn diff_truncated_screen_clears_stale_rows() {
        let mut r = renderer(80, 24, false, ResizeScrollbackMode::Preserve);
        let full: Vec<String> = vec!["a", "b", "c", "d", "e"]
            .into_iter().map(String::from).collect();
        r.draw(FramePlan { history: None, viewport: full }).unwrap();

        r.out_mut().data.clear();
        let short: Vec<String> = vec!["a", "b"].into_iter().map(String::from).collect();
        r.draw(FramePlan { history: None, viewport: short }).unwrap();

        let diff = r.out().output();
        assert!(diff.contains("\x1b[3H\x1b[2K"), "clear stale row 3");
        assert!(diff.contains("\x1b[4H\x1b[2K"), "clear stale row 4");
        assert!(diff.contains("\x1b[5H\x1b[2K"), "clear stale row 5");
    }

    // ---- Synchronized output ----------------------------------------------

    #[test]
    fn sync_output_wraps_viewport_updates() {
        let mut r = renderer(80, 24, true, ResizeScrollbackMode::Preserve);
        r.draw(FramePlan {
            history: None,
            viewport: vec!["hello".into()],
        })
        .unwrap();

        let output = r.out().output();
        // Golden ANSI: sync begin … writes … sync end, cursor park inside.
        assert!(output.starts_with("\x1b[?2026h"), "sync begin at start");
        assert!(output.ends_with("\x1b[?2026l"), "sync end at end");
        assert!(output.contains("\x1b[1Hhello"), "viewport write inside sync");
        assert!(output.contains("\x1b[2H"), "cursor park inside sync");
    }

    #[test]
    fn sync_output_disabled_omits_markers() {
        let mut r = renderer(80, 24, false, ResizeScrollbackMode::Preserve);
        r.draw(FramePlan {
            history: None,
            viewport: vec!["hello".into()],
        })
        .unwrap();
        let output = r.out().output();
        assert!(!output.contains("\x1b[?2026h"), "no sync begin");
        assert!(!output.contains("\x1b[?2026l"), "no sync end");
    }

    // ---- Cursor park ------------------------------------------------------

    #[test]
    fn cursor_parked_at_viewport_bottom() {
        let mut r = renderer(80, 24, false, ResizeScrollbackMode::Preserve);
        let rows: Vec<String> = vec!["a", "b", "c"].into_iter().map(String::from).collect();
        r.draw(FramePlan { history: None, viewport: rows }).unwrap();
        let output = r.out().output();
        // 3 viewport rows → park at row 4.
        assert!(output.ends_with("\x1b[4H"), "cursor parked at row 4");
    }

    #[test]
    fn no_changes_skips_park_and_sync() {
        let mut r = renderer(80, 24, true, ResizeScrollbackMode::Preserve);
        let rows: Vec<String> = vec!["same"].into_iter().map(String::from).collect();
        r.draw(FramePlan { history: None, viewport: rows.clone() }).unwrap();
        r.out_mut().data.clear();
        r.draw(FramePlan { history: None, viewport: rows }).unwrap();
        assert!(r.out().output().is_empty(), "identical frame emits nothing");
    }

    // ---- Overlays ---------------------------------------------------------

    #[test]
    fn overlay_show_and_hide_never_writes_history() {
        let mut r = renderer(80, 10, false, ResizeScrollbackMode::Preserve);
        let base: Vec<String> = (0..10).map(|i| format!("row{i}")).collect();

        // Frame 1: show overlay on top of base viewport.
        let handle = r.show_overlay(
            Box::new(Mock::new(&["OVERLAY1", "OVERLAY2"])),
            Anchor::BottomCenter,
        );
        r.draw(FramePlan { history: None, viewport: base.clone() }).unwrap();

        // Frame 2: overlay still shown — different content.
        r.draw(FramePlan { history: None, viewport: base.clone() }).unwrap();

        // Frame 3: hide overlay.
        r.hide_overlay(handle);
        r.draw(FramePlan { history: None, viewport: base }).unwrap();

        // No history rows were ever written: history writes are
        // newline-terminated; viewport writes are cursor-positioned.
        let output = r.out().output();
        assert!(!output.contains("row0\n"), "no history rows written");
        assert!(
            !output.contains("OVERLAY\n"),
            "overlay rows never written as history"
        );
        assert!(output.contains("\x1b[9H"), "overlay composited into viewport");
    }

    #[test]
    fn overlay_rows_removed_after_close() {
        let mut r = renderer(80, 10, false, ResizeScrollbackMode::Preserve);
        let base: Vec<String> = (0..10).map(|i| format!("row{i}")).collect();

        let handle = r.show_overlay(
            Box::new(Mock::new(&["OVERLAY1", "OVERLAY2"])),
            Anchor::BottomCenter,
        );
        r.draw(FramePlan { history: None, viewport: base.clone() }).unwrap();

        r.out_mut().data.clear();
        r.hide_overlay(handle);
        r.draw(FramePlan { history: None, viewport: base }).unwrap();

        // After close, the frame reverts to the pure viewport: the overlay
        // rows are replaced by the underlying viewport rows, and nothing is
        // written to history.
        let diff = r.out().output();
        assert!(diff.contains("\x1b[9Hrow8"), "overlay row 1 reverted");
        assert!(diff.contains("\x1b[10Hrow9"), "overlay row 2 reverted");
        assert!(!diff.contains("OVERLAY"), "overlay content gone after close");
    }

    // ---- Resize policies --------------------------------------------------

    #[test]
    fn resize_preserve_writes_no_history() {
        let mut r = renderer(80, 24, false, ResizeScrollbackMode::Preserve);

        // Seed history.
        r.draw(FramePlan {
            history: Some(hb(1, &["h1", "h2"], BatchKind::Append)),
            viewport: vec!["v1".into()],
        })
        .unwrap();

        let mut provider = TestProvider::new(FramePlan {
            history: Some(hb(2, &["h1", "h2"], BatchKind::Replay)),
            viewport: vec!["v1".into()],
        });

        r.out_mut().data.clear();
        r.on_resize(100, 30, &mut provider).unwrap();

        let output = r.out().output();
        // No ED3, no replay rows in Preserve mode — only viewport redraw.
        assert!(!output.contains("\x1b[3J"), "no ED3 in Preserve");
        assert!(!output.contains("h1"), "no history replay in Preserve");
        assert!(output.contains("\x1b[1Hv1"), "viewport repainted");
    }

    #[test]
    fn resize_append_replays_history_once() {
        let mut r = renderer(80, 24, false, ResizeScrollbackMode::Append);

        r.draw(FramePlan {
            history: Some(hb(1, &["h1", "h2"], BatchKind::Append)),
            viewport: vec!["v1".into()],
        })
        .unwrap();

        let mut provider = TestProvider::new(FramePlan {
            history: Some(hb(2, &["h1", "h2", "v1"], BatchKind::Replay)),
            viewport: vec!["v1".into()],
        });

        r.out_mut().data.clear();
        r.on_resize(100, 30, &mut provider).unwrap();

        let output = r.out().output();
        // Append: no ED3; replay batch written once below existing history.
        assert!(!output.contains("\x1b[3J"), "no ED3 in Append");
        assert!(output.contains("h1"), "history replayed");
        assert_eq!(output.matches("h1").count(), 1, "replayed exactly once");
        assert!(provider.acked.contains(&2), "ack after write");
    }

    #[test]
    fn resize_rebuild_ed3_then_replay() {
        let mut r = renderer(80, 24, false, ResizeScrollbackMode::Rebuild);

        r.draw(FramePlan {
            history: Some(hb(1, &["h1", "h2"], BatchKind::Append)),
            viewport: vec!["v1".into()],
        })
        .unwrap();

        let mut provider = TestProvider::new(FramePlan {
            history: Some(hb(2, &["h1", "h2", "v1"], BatchKind::Replay)),
            viewport: vec!["v1".into()],
        });

        r.out_mut().data.clear();
        r.on_resize(100, 30, &mut provider).unwrap();

        let output = r.out().output();
        // Rebuild: ED3 first, then full replay.
        assert!(output.starts_with("\x1b[3J"), "ED3 clears native history");
        assert!(output.contains("h1"), "history replayed");
        assert!(provider.acked.contains(&2), "ack after write");
    }

    // ---- Reset ------------------------------------------------------------

    #[test]
    fn reset_display_reoffers_under_new_ids() {
        let mut r = renderer(80, 24, false, ResizeScrollbackMode::Preserve);

        r.draw(FramePlan {
            history: Some(hb(1, &["old"], BatchKind::Append)),
            viewport: vec!["v".into()],
        })
        .unwrap();

        // After reset, provider offers the same content under a fresh id.
        let mut provider = TestProvider::new(FramePlan {
            history: Some(hb(100, &["old"], BatchKind::Replay)),
            viewport: vec!["v".into()],
        });

        r.out_mut().data.clear();
        r.reset_display(&mut provider).unwrap();

        let output = r.out().output();
        assert!(output.starts_with("\x1b[3J"), "reset clears native history");
        assert!(provider.acked.contains(&100), "re-offered batch acked");
    }

    // ---- Finality is the application's decision ---------------------------

    #[test]
    fn unacked_rows_never_enter_history() {
        // A block that crossed the viewport top without provider finalization
        // must never be written as history: the renderer only writes what the
        // provider offers in a batch.
        let mut r = renderer(80, 24, false, ResizeScrollbackMode::Preserve);
        let plan = FramePlan {
            history: None, // provider did NOT finalize anything
            viewport: vec!["floating".into()],
        };
        r.draw(plan).unwrap();
        let output = r.out().output();
        assert!(!output.contains("floating\n"), "no history write without finalization");
        assert!(output.contains("\x1b[1Hfloating"), "viewport-only render");
    }
}