//! Prewalk: one-shot handoff state machine.
//!
//! Arm → the first successful `todo` call opens the gate → the first
//! `edit`/`write` performs the model switch and disarms. Re-arming is possible
//! only after a completed Disarm; one-shot semantics throughout.

/// Cheap-model target a prewalk hands off to, e.g. `@smol`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrewalkTarget {
    pub model: String,
}

impl PrewalkTarget {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
        }
    }
}

/// Lifecycle states of the prewalk handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrewalkState {
    /// Armed with a handoff target; waiting for a successful todo.
    Armed { target: PrewalkTarget },
    /// Todo succeeded; the next edit/write triggers the switch.
    GateOpen,
    /// Handoff consumed or never armed; inert until re-armed.
    Disarmed,
}

impl PrewalkState {
    pub fn as_str(self) -> &'static str {
        match self {
            PrewalkState::Armed { .. } => "armed",
            PrewalkState::GateOpen => "gate-open",
            PrewalkState::Disarmed => "disarmed",
        }
    }
}

/// What to do when a transition fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrewalkEvent {
    /// Gate opened after a successful todo; no switch yet.
    GateOpened,
    /// Perform the model switch to this target, then disarm.
    Switch(PrewalkTarget),
}

/// State machine driving the one-shot prewalk handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prewalk {
    /// Armed with a handoff target; waiting for a successful todo.
    Armed { target: PrewalkTarget },
    /// Todo succeeded; the next edit/write triggers the switch.
    GateOpen { target: PrewalkTarget },
    /// Handoff consumed or never armed; inert until re-armed.
    Disarmed,
}

impl Prewalk {
    /// A fresh machine in the Disarmed state.
    pub fn new() -> Self {
        Prewalk::Disarmed
    }

    /// Arm with a target. Fails while not Disarmed — re-arm is possible only
    /// after a completed disarm.
    pub fn arm(&mut self, target: PrewalkTarget) -> Result<(), PrewalkError> {
        if !matches!(self, Prewalk::Disarmed) {
            return Err(PrewalkError::AlreadyArmed);
        }
        *self = Prewalk::Armed { target };
        Ok(())
    }

    /// Record a successful `todo` call (read-only `view` counts too). In
    /// `Armed`, opens the gate.
    pub fn on_todo_success(&mut self) -> Option<PrewalkEvent> {
        let target = match self {
            Prewalk::Armed { target } => Some(target.clone()),
            _ => None,
        }?;
        *self = Prewalk::GateOpen { target };
        Some(PrewalkEvent::GateOpened)
    }

    /// Record an `edit`/`write` call. In `GateOpen`, returns the switch target
    /// and disarms. One-shot: later calls return `None` until re-armed.
    pub fn on_edit_write(&mut self) -> Option<PrewalkTarget> {
        let target = match self {
            Prewalk::GateOpen { target } => Some(target.clone()),
            _ => None,
        }?;
        *self = Prewalk::Disarmed;
        Some(target)
    }

    /// Current lifecycle state.
    pub fn state(&self) -> PrewalkState {
        match self {
            Prewalk::Armed { target } => PrewalkState::Armed {
                target: target.clone(),
            },
            Prewalk::GateOpen { .. } => PrewalkState::GateOpen,
            Prewalk::Disarmed => PrewalkState::Disarmed,
        }
    }

    /// True while armed or gated (handoff still pending).
    pub fn is_active(&self) -> bool {
        !matches!(self, Prewalk::Disarmed)
    }
}

impl Default for Prewalk {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors surfaced by the prewalk state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrewalkError {
    /// Arm attempted while the machine was not Disarmed.
    AlreadyArmed,
}

impl std::fmt::Display for PrewalkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrewalkError::AlreadyArmed => write!(f, "prewalk already armed"),
        }
    }
}

impl std::error::Error for PrewalkError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only `expect` for Results; `expect` itself is forbidden in src/.
    trait ExpectInTest<T> {
        fn expect_in_test(self, msg: &str) -> T;
    }

    impl<T, E: std::fmt::Debug> ExpectInTest<T> for Result<T, E> {
        fn expect_in_test(self, msg: &str) -> T {
            match self {
                Ok(v) => v,
                Err(e) => panic!("{msg}: {e:?}"),
            }
        }
    }

    #[test]
    fn one_shot_full_lifecycle() {
        let mut prewalk = Prewalk::new();
        assert_eq!(prewalk.state(), PrewalkState::Disarmed);

        prewalk
            .arm(PrewalkTarget::new("@smol"))
            .expect_in_test("arm on disarmed");
        assert_eq!(
            prewalk.state(),
            PrewalkState::Armed {
                target: PrewalkTarget::new("@smol")
            }
        );

        // Edits before a todo do not switch.
        assert_eq!(prewalk.on_edit_write(), None);

        assert_eq!(
            prewalk.on_todo_success(),
            Some(PrewalkEvent::GateOpened),
            "todo opens the gate"
        );
        assert_eq!(prewalk.state(), PrewalkState::GateOpen);

        // More todos while gated are inert.
        assert_eq!(prewalk.on_todo_success(), None);

        let switch = prewalk.on_edit_write();
        assert_eq!(switch, Some(PrewalkTarget::new("@smol")));
        assert_eq!(prewalk.state(), PrewalkState::Disarmed);

        // One-shot: nothing fires until re-armed.
        assert_eq!(prewalk.on_todo_success(), None);
        assert_eq!(prewalk.on_edit_write(), None);
    }

    #[test]
    fn rearm_only_after_disarm() {
        let mut prewalk = Prewalk::new();
        prewalk
            .arm(PrewalkTarget::new("cheap-a"))
            .expect_in_test("first arm");
        assert_eq!(
            prewalk.arm(PrewalkTarget::new("cheap-b")),
            Err(PrewalkError::AlreadyArmed),
            "double arm rejected"
        );

        prewalk.on_todo_success();
        assert_eq!(
            prewalk.arm(PrewalkTarget::new("cheap-b")),
            Err(PrewalkError::AlreadyArmed),
            "arm while gated rejected"
        );

        assert!(prewalk.on_edit_write().is_some());
        assert_eq!(prewalk.state(), PrewalkState::Disarmed);
        prewalk
            .arm(PrewalkTarget::new("cheap-b"))
            .expect_in_test("re-arm after disarm");
        assert!(prewalk.is_active());
    }

    #[test]
    fn gate_opens_on_read_only_todo_view() {
        // Read-only todo (`view`) still counts as a successful todo call.
        let mut prewalk = Prewalk::default();
        prewalk
            .arm(PrewalkTarget::new("@smol"))
            .expect_in_test("arm");
        assert_eq!(prewalk.on_todo_success(), Some(PrewalkEvent::GateOpened));
        assert_eq!(prewalk.state(), PrewalkState::GateOpen);
    }

    #[test]
    fn events_on_disarmed_are_inert() {
        let mut prewalk = Prewalk::new();
        assert_eq!(prewalk.on_todo_success(), None);
        assert_eq!(prewalk.on_edit_write(), None);
        assert!(!prewalk.is_active());
    }

    #[test]
    fn state_names_are_stable() {
        assert_eq!(PrewalkState::Disarmed.as_str(), "disarmed");
        assert_eq!(PrewalkState::GateOpen.as_str(), "gate-open");
    }
}
