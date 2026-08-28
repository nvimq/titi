//! Focus router: owns the component registry and delivers input to the
//! focused component (contract: `omp://tui` — `Focusable` split, spec:
//! `docs/research/tui-renderer/README.md` §3).

use crate::component::Component;

/// Stable handle to a registered component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FocusHandle(usize);

struct Slot {
    id: usize,
    component: Box<dyn Component>,
}

/// Registry of components with at most one focused member; input is routed
/// to the focused component only.
#[derive(Default)]
pub struct FocusRouter {
    slots: Vec<Slot>,
    next_id: usize,
    focused: Option<usize>,
}

impl FocusRouter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `component`; starts unfocused. Returns its handle.
    pub fn add(&mut self, component: Box<dyn Component>) -> FocusHandle {
        let handle = FocusHandle(self.next_id);
        self.next_id += 1;
        self.slots.push(Slot {
            id: handle.0,
            component,
        });
        handle
    }

    /// Remove a registered component; returns it, or `None` for an unknown
    /// handle. Removing the focused component blurs the router.
    pub fn remove(&mut self, handle: FocusHandle) -> Option<Box<dyn Component>> {
        let index = self.slots.iter().position(|s| s.id == handle.0)?;
        if self.focused == Some(handle.0) {
            self.focused = None;
        }
        let slot = self.slots.remove(index);
        Some(slot.component)
    }

    /// Focus the component referenced by `handle`. Returns `false` (and
    /// leaves focus untouched) for an unknown handle.
    pub fn focus(&mut self, handle: FocusHandle) -> bool {
        match self.slots.iter().position(|s| s.id == handle.0) {
            Some(_) => {
                self.focused = Some(handle.0);
                true
            }
            None => false,
        }
    }

    /// Drop focus; no component receives input until the next `focus`.
    pub fn blur(&mut self) {
        self.focused = None;
    }

    /// Handle of the focused component, if any.
    pub fn focused(&self) -> Option<FocusHandle> {
        self.focused.map(FocusHandle)
    }

    /// Whether `handle` refers to the currently focused component.
    pub fn has_focus(&self, handle: FocusHandle) -> bool {
        self.focused == Some(handle.0)
    }

    /// Deliver raw input to the focused component. Returns `false` when
    /// nothing is focused.
    pub fn route_input(&mut self, data: &str) -> bool {
        let Some(id) = self.focused else {
            return false;
        };
        match self.slots.iter_mut().find(|s| s.id == id) {
            // Unreachable while the invariant holds (remove clears focus),
            // but a lost slot must never panic the render loop.
            Some(slot) => {
                slot.component.handle_input(data);
                true
            }
            None => false,
        }
    }

    /// Borrow a registered component for rendering.
    pub fn component(&self, handle: FocusHandle) -> Option<&(dyn Component + '_)> {
        self.slots
            .iter()
            .find(|s| s.id == handle.0)
            .map(|s| s.component.as_ref())
    }

    /// Mutably borrow a registered component for rendering.
    pub fn component_mut(&mut self, handle: FocusHandle) -> Option<&mut (dyn Component + '_)> {
        let slot = self.slots.iter_mut().find(|s| s.id == handle.0)?;
        Some(slot.component.as_mut())
    }

    /// Number of registered components.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether no component is registered.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Recorder logging every delivered input into a shared buffer the test
    /// keeps beside the boxed component.
    struct Recorder {
        log: Rc<RefCell<Vec<String>>>,
        rows: Vec<String>,
    }

    impl Recorder {
        fn new() -> (Self, Rc<RefCell<Vec<String>>>) {
            let log = Rc::new(RefCell::new(Vec::new()));
            (
                Recorder {
                    log: Rc::clone(&log),
                    rows: vec!["row".to_owned()],
                },
                log,
            )
        }
    }

    impl Component for Recorder {
        fn render(&mut self, _width: u16) -> Vec<String> {
            self.rows.clone()
        }

        fn handle_input(&mut self, data: &str) {
            self.log.borrow_mut().push(data.to_owned());
        }
    }

    #[test]
    fn router_starts_empty_and_unfocused() {
        let router = FocusRouter::new();
        assert!(router.is_empty());
        assert_eq!(router.focused(), None);
        assert_eq!(router.len(), 0);
    }

    #[test]
    fn input_is_dropped_when_nothing_is_focused() {
        let mut router = FocusRouter::new();
        let (rec, log) = Recorder::new();
        let h = router.add(Box::new(rec));
        assert!(!router.route_input("k"));
        assert!(!router.has_focus(h));
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn input_goes_to_the_focused_component_only() {
        let mut router = FocusRouter::new();
        let (rec_a, log_a) = Recorder::new();
        let (rec_b, log_b) = Recorder::new();
        let a = router.add(Box::new(rec_a));
        let b = router.add(Box::new(rec_b));

        assert!(router.focus(a));
        assert!(router.route_input("x"));
        assert!(router.route_input("y"));
        assert_eq!(*log_a.borrow(), vec!["x".to_owned(), "y".to_owned()]);
        assert!(log_b.borrow().is_empty());

        assert!(router.focus(b));
        assert!(router.route_input("z"));
        assert_eq!(*log_a.borrow(), vec!["x".to_owned(), "y".to_owned()]);
        assert_eq!(*log_b.borrow(), vec!["z".to_owned()]);
    }

    #[test]
    fn focus_moves_between_components() {
        let mut router = FocusRouter::new();
        let (rec_a, _) = Recorder::new();
        let (rec_b, _) = Recorder::new();
        let a = router.add(Box::new(rec_a));
        let b = router.add(Box::new(rec_b));

        assert!(router.focus(a));
        assert_eq!(router.focused(), Some(a));
        assert!(router.focus(b));
        assert_eq!(router.focused(), Some(b));
        assert!(!router.has_focus(a));
        assert!(router.has_focus(b));
    }

    #[test]
    fn unknown_handle_is_rejected_without_side_effects() {
        let mut router = FocusRouter::new();
        let (rec, _) = Recorder::new();
        let a = router.add(Box::new(rec));
        assert!(router.focus(a));

        let ghost = FocusHandle(999);
        assert!(!router.focus(ghost));
        assert_eq!(router.focused(), Some(a));
        assert!(!router.has_focus(ghost));
        assert!(router.remove(ghost).is_none());
        assert!(router.component(ghost).is_none());
    }

    #[test]
    fn blur_stops_input_delivery() {
        let mut router = FocusRouter::new();
        let (rec, log) = Recorder::new();
        let a = router.add(Box::new(rec));
        assert!(router.focus(a));
        router.blur();
        assert_eq!(router.focused(), None);
        assert!(!router.has_focus(a));
        assert!(!router.route_input("k"));
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn removing_the_focused_component_blurs() {
        let mut router = FocusRouter::new();
        let (rec, log) = Recorder::new();
        let a = router.add(Box::new(rec));
        assert!(router.focus(a));

        router.remove(a).expect("a registered");
        assert!(router.is_empty());
        assert_eq!(router.focused(), None);
        assert!(!router.route_input("k"));
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn removing_an_unfocused_component_keeps_focus() {
        let mut router = FocusRouter::new();
        let (rec_a, _) = Recorder::new();
        let (rec_b, log_b) = Recorder::new();
        let a = router.add(Box::new(rec_a));
        let b = router.add(Box::new(rec_b));
        assert!(router.focus(b));

        router.remove(a).expect("a registered");
        assert_eq!(router.len(), 1);
        assert!(router.has_focus(b));
        assert!(router.route_input("k"));
        assert_eq!(*log_b.borrow(), vec!["k".to_owned()]);
    }

    #[test]
    fn add_after_remove_keeps_handles_unique() {
        let mut router = FocusRouter::new();
        let (rec_a, _) = Recorder::new();
        let (rec_b, _) = Recorder::new();
        let a = router.add(Box::new(rec_a));
        router.remove(a).expect("a registered");
        let b = router.add(Box::new(rec_b));
        assert_ne!(a, b);
        assert!(router.focus(b));
        assert!(!router.focus(a)); // stale handle of the removed slot
    }

    #[test]
    fn registered_components_render_through_the_router() {
        let mut router = FocusRouter::new();
        let (rec, _) = Recorder::new();
        let a = router.add(Box::new(rec));

        let rows = router.component_mut(a).expect("a registered").render(20);
        assert_eq!(rows, vec!["row".to_owned()]);
        assert!(router.component(FocusHandle(999)).is_none());
    }
}
