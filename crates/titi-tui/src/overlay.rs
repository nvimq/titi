//! Overlay stack: modal components composited on top of the viewport frame.
//! Overlays are viewport-local — showing, updating or hiding one never
//! creates history
//! (contract: `omp://tui-core-renderer`,
//! spec: `docs/research/tui-renderer/frame-history.md` §4).

use crate::component::Component;
use crate::width;

/// Where an overlay sits inside the viewport. v1 anchors bottom-center only;
/// the enum exists so call sites and storage stay forward-compatible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// Bottom edge of the viewport, horizontally centered.
    BottomCenter,
    /// Top edge of the viewport, horizontally centered.
    TopCenter,
}

/// Stable handle to a shown overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverlayHandle(u64);

struct Overlay {
    id: u64,
    component: Box<dyn Component>,
    anchor: Anchor,
}
#[derive(Default)]
pub struct OverlayStack {
    next_id: u64,
    overlays: Vec<Overlay>,
}

impl OverlayStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Show `component` anchored at `anchor`; returns its handle.
    pub fn show(&mut self, component: Box<dyn Component>, anchor: Anchor) -> OverlayHandle {
        let handle = OverlayHandle(self.next_id);
        self.next_id += 1;
        self.overlays.push(Overlay {
            id: handle.0,
            component,
            anchor,
        });
        handle
    }

    /// Hide the overlay referenced by `handle`; returns its component, or
    /// `None` when the handle is unknown / already hidden.
    pub fn hide(&mut self, handle: OverlayHandle) -> Option<Box<dyn Component>> {
        let index = self.overlays.iter().position(|o| o.id == handle.0)?;
        let overlay = self.overlays.remove(index);
        Some(overlay.component)
    }

    /// Number of shown overlays.
    pub fn len(&self) -> usize {
        self.overlays.len()
    }

    /// Whether no overlay is shown.
    pub fn is_empty(&self) -> bool {
        self.overlays.is_empty()
    }

    /// Handle of the topmost (last shown) overlay, if any.
    pub fn top_handle(&self) -> Option<OverlayHandle> {
        self.overlays.last().map(|o| OverlayHandle(o.id))
    }

    /// Deliver raw input to the topmost overlay (modal: overlays sit above
    /// the focus chain). Returns `false` when no overlay is shown.
    pub fn handle_input(&mut self, data: &str) -> bool {
        match self.overlays.last_mut() {
            Some(overlay) => {
                overlay.component.handle_input(data);
                true
            }
            None => false,
        }
    }

    /// Composite every overlay on top of `viewport` at `width` and return
    /// the blended frame. An anchored overlay row fully replaces the
    /// viewport line it lands on; rows the overlay does not reach are passed
    /// through unchanged. Overlay rows overflowing the top of the frame are
    /// dropped, so the frame height never grows.
    pub fn composite(&mut self, viewport: &[String], width: u16) -> Vec<String> {
        let mut frame = viewport.to_vec();
        for overlay in &mut self.overlays {
            let rows = overlay.component.render(width);
            if rows.is_empty() {
                continue;
            }
            frame = composite_rows(&frame, &rows, width, overlay.anchor);
        }
        frame
    }
}

/// Composite one overlay's rows over the viewport: each row fully replaces
/// the bottom-aligned frame line it lands on; rows the overlay does not
/// reach pass through unchanged; rows overflowing the top of the frame are
/// dropped, so the frame height never grows.
///
/// Shared by [`OverlayStack::composite`] and application-level
/// single-overlay composition (the typed-panel path in `titi-cli`).
pub fn composite_rows(
    viewport: &[String],
    rows: &[String],
    width: u16,
    anchor: Anchor,
) -> Vec<String> {
    let mut frame = viewport.to_vec();
    let n = rows.len();
    let overflow_n = n.saturating_sub(frame.len());
    for (i, row) in rows.iter().enumerate() {
        let target = match anchor {
            // Bottom-anchored: last rows stay, top rows overflow.
            Anchor::BottomCenter => {
                if i < overflow_n {
                    continue;
                }
                frame.len() - (n - i)
            }
            // Top-anchored: first rows stay, bottom rows overflow.
            Anchor::TopCenter => {
                if i >= frame.len() {
                    // Row past the bottom of the viewport — drop it.
                    break;
                }
                i
            }
        };
        frame[target] = anchored_line(row, width, anchor);
    }
    frame
}

/// Lay a single overlay row into a full-width frame line for `anchor`.
fn anchored_line(row: &str, width: u16, _anchor: Anchor) -> String {
    let cells = width as usize;
    let visible = width::visible_width(row);
    if visible >= cells {
        return width::truncate_to_width(row, cells);
    }
    let pad = (cells - visible) / 2;
    let mut line = String::with_capacity(cells);
    for _ in 0..pad {
        line.push(' ');
    }
    line.push_str(row);
    for _ in 0..(cells - visible - pad) {
        line.push(' ');
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Static {
        rows: Vec<String>,
        input: Vec<String>,
    }

    impl Static {
        fn new(rows: &[&str]) -> Self {
            Static {
                rows: rows.iter().map(|s| (*s).to_owned()).collect(),
                input: Vec::new(),
            }
        }
    }

    impl Component for Static {
        fn render(&mut self, _width: u16) -> Vec<String> {
            self.rows.clone()
        }

        fn handle_input(&mut self, data: &str) {
            self.input.push(data.to_owned());
        }
    }

    fn viewport(rows: &[&str]) -> Vec<String> {
        rows.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn empty_stack_passes_viewport_through() {
        let mut stack = OverlayStack::new();
        let vp = viewport(&["one", "two"]);
        assert!(stack.is_empty());
        assert_eq!(stack.composite(&vp, 40), vp);
    }

    #[test]
    fn show_returns_handle_and_hide_removes() {
        let mut stack = OverlayStack::new();
        let h = stack.show(Box::new(Static::new(&["ov"])), Anchor::BottomCenter);
        assert_eq!(stack.len(), 1);
        assert_eq!(stack.top_handle(), Some(h));

        stack.hide(h).expect("overlay present");
        assert!(stack.is_empty());
        assert_eq!(stack.top_handle(), None);

        // Second hide of the same handle: unknown.
        assert!(stack.hide(h).is_none());
    }

    #[test]
    fn bottom_center_overlay_replaces_the_bottom_rows() {
        let mut stack = OverlayStack::new();
        stack.show(Box::new(Static::new(&["hello"])), Anchor::BottomCenter);
        let vp = viewport(&["line0", "line1", "line2"]);
        let frame = stack.composite(&vp, 11);

        assert_eq!(frame.len(), 3);
        assert_eq!(frame[0], "line0");
        assert_eq!(frame[1], "line1");
        // 5 visible cells in 11 → 3 spaces left, 3 right.
        assert_eq!(frame[2], "   hello   ");
    }

    #[test]
    fn multi_row_overlay_occupies_a_bottom_block() {
        let mut stack = OverlayStack::new();
        stack.show(
            Box::new(Static::new(&["first", "second"])),
            Anchor::BottomCenter,
        );
        let vp = viewport(&["a", "b", "c", "d"]);
        let frame = stack.composite(&vp, 8);
        assert_eq!(
            frame,
            vec!["a".to_owned(), "b".to_owned(), " first  ".to_owned(), " second ".to_owned()]
        );
    }

    #[test]
    fn centering_uses_visible_width_not_bytes() {
        let mut stack = OverlayStack::new();
        stack.show(
            Box::new(Static::new(&["\x1b[31mhi\x1b[0m"])),
            Anchor::BottomCenter,
        );
        let vp = viewport(&["x"]);
        let frame = stack.composite(&vp, 6);
        // "hi" is 2 cells of 6 → 2 spaces each side; escapes zero-width.
        assert_eq!(frame, vec![format!("  \x1b[31mhi\x1b[0m  ")]);
    }

    #[test]
    fn wide_chars_center_by_cells() {
        let mut stack = OverlayStack::new();
        stack.show(Box::new(Static::new(&["日本"])), Anchor::BottomCenter);
        let vp = viewport(&["x"]);
        let frame = stack.composite(&vp, 8);
        // 4 cells of 8 → 2 left, 2 right.
        assert_eq!(frame, vec!["  日本  ".to_owned()]);
    }

    #[test]
    fn overlay_wider_than_frame_is_truncated_not_wrapped() {
        let mut stack = OverlayStack::new();
        stack.show(Box::new(Static::new(&["abcdefgh"])), Anchor::BottomCenter);
        let vp = viewport(&["x"]);
        let frame = stack.composite(&vp, 4);
        assert_eq!(frame, vec!["abcd".to_owned()]);
    }

    #[test]
    fn overlay_taller_than_frame_drops_top_overflow() {
        let mut stack = OverlayStack::new();
        stack.show(
            Box::new(Static::new(&["one", "two", "three"])),
            Anchor::BottomCenter,
        );
        let vp = viewport(&["v0", "v1"]);
        let frame = stack.composite(&vp, 6);

        // Frame height stays 2; the bottom two overlay rows win.
        assert_eq!(frame.len(), 2);
        assert_eq!(frame[0], " two  ");
        assert_eq!(frame[1], "three ");
    }
    #[test]
    fn top_anchored_overlay_lands_on_the_top_rows() {
        let mut stack = OverlayStack::new();
        stack.show(
            Box::new(Static::new(&["one", "two", "three"])),
            Anchor::TopCenter,
        );
        let vp = viewport(&["v0", "v1", "v2"]);
        let frame = stack.composite(&vp, 6);

        // Top three rows replaced; height stays 3.
        assert_eq!(frame.len(), 3);
        assert_eq!(frame[0], " one  ");
        assert_eq!(frame[1], " two  ");
        assert_eq!(frame[2], "three ");
    }

    #[test]
    fn top_anchored_overlay_taller_than_frame_drops_bottom_overflow() {
        let mut stack = OverlayStack::new();
        stack.show(
            Box::new(Static::new(&["one", "two", "three"])),
            Anchor::TopCenter,
        );
        let vp = viewport(&["v0", "v1"]);
        let frame = stack.composite(&vp, 6);

        // Frame height stays 2; the top two overlay rows win.
        assert_eq!(frame.len(), 2);
        assert_eq!(frame[0], " one  ");
        assert_eq!(frame[1], " two  ");
    }

    #[test]
    fn later_overlay_composites_above_earlier() {
        let mut stack = OverlayStack::new();
        stack.show(Box::new(Static::new(&["second"])), Anchor::BottomCenter);
        let vp = viewport(&["a", "b"]);
        let frame = stack.composite(&vp, 8);
        assert_eq!(frame, vec!["a".to_owned(), " second ".to_owned()]);
    }

    #[test]
    fn input_goes_to_the_topmost_overlay_only() {
        use std::cell::RefCell;
        use std::rc::Rc;

        /// Logs every delivered input into a shared buffer.
        struct Logging(Rc<RefCell<Vec<String>>>);
        impl Component for Logging {
            fn render(&mut self, _width: u16) -> Vec<String> {
                vec!["x".to_owned()]
            }
            fn handle_input(&mut self, data: &str) {
                self.0.borrow_mut().push(data.to_owned());
            }
        }

        let bottom_log = Rc::new(RefCell::new(Vec::new()));
        let top_log = Rc::new(RefCell::new(Vec::new()));
        let mut stack = OverlayStack::new();
        stack.show(
            Box::new(Logging(Rc::clone(&bottom_log))),
            Anchor::BottomCenter,
        );
        stack.show(Box::new(Logging(Rc::clone(&top_log))), Anchor::BottomCenter);

        assert!(stack.handle_input("k"));
        assert!(bottom_log.borrow().is_empty());
        assert_eq!(*top_log.borrow(), vec!["k".to_owned()]);

        // An empty stack consumes nothing.
        assert!(!OverlayStack::new().handle_input("k"));
    }
    #[test]
    fn hide_restores_the_viewport_line() {
        let mut stack = OverlayStack::new();
        let h = stack.show(Box::new(Static::new(&["ov"])), Anchor::BottomCenter);
        let vp = viewport(&["under"]);
        let with_overlay = stack.composite(&vp, 4);
        assert_eq!(with_overlay, vec![" ov ".to_owned()]);

        stack.hide(h);
        assert_eq!(stack.composite(&vp, 4), vp);
    }

    #[test]
    fn zero_width_frame_empties_overlay_rows() {
        let mut stack = OverlayStack::new();
        stack.show(Box::new(Static::new(&["ab"])), Anchor::BottomCenter);
        let frame = stack.composite(&viewport(&["x"]), 0);
        assert_eq!(frame, vec![String::new()]);
    }
}
