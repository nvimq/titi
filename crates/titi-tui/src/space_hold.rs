//! Space-hold push-to-talk gesture (omp custom-editor `#handleSpaceHold`).
//!
//! A held space bar emits OS auto-repeat: a *steady* stream of spaces at a
//! fixed fast interval. Inter-space deltas are classified as mechanical when
//! both are auto-repeat-fast and near-identical. Smashing the bar is fast but
//! jittery and deliberate taps are too slow, so neither escalates; both keep
//! typing real spaces. The few spaces typed before a real hold is recognized
//! are tracked back out.
//!
//! The CLI event loop polls [`SpaceHold::poll`] on the idle 50ms tick so the
//! 250ms release timeout fires without a dedicated thread.

use std::time::{Duration, Instant};

/// Max gap (ms) between two spaces for the later one to count as OS auto-repeat.
pub const SPACE_REPEAT_MAX_GAP_MS: u64 = 120;
/// Absolute jitter floor (ms) for two consecutive inter-space gaps.
pub const SPACE_REPEAT_JITTER_MS: u64 = 18;
/// Proportional jitter of the smaller gap (slower repeat rates).
pub const SPACE_REPEAT_JITTER_RATIO: f64 = 0.35;
/// Consecutive mechanical deltas that confirm the space bar is held.
pub const SPACE_HOLD_MECHANICAL_RUN: u32 = 2;
/// Idle gap after the last repeated space that counts as release.
pub const SPACE_HOLD_RELEASE_MS: u64 = 250;

/// Result of feeding one key into the space-hold machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceHoldOutcome {
    /// Not handled; caller continues normal key dispatch.
    Continue,
    /// Insert a real space (first tap / jitter / slow press).
    InsertSpace,
    /// Swallow the key (repeat while deciding or while held).
    Swallow,
    /// Hold recognized. Retract `retract` characters, then start recording.
    Start { retract: usize },
    /// Hold ended (non-space while held). Caller then dispatches the key.
    EndThenContinue,
}

/// Whether two consecutive inter-space gaps look machine-driven.
pub fn gaps_are_mechanical(gap: Duration, prev_gap: Duration) -> bool {
    let max = Duration::from_millis(SPACE_REPEAT_MAX_GAP_MS);
    if gap > max || prev_gap > max {
        return false;
    }
    let smaller = gap.min(prev_gap).as_secs_f64() * 1000.0;
    let tolerance_ms = (SPACE_REPEAT_JITTER_MS as f64).max(smaller * SPACE_REPEAT_JITTER_RATIO);
    let delta = gap.abs_diff(prev_gap).as_secs_f64() * 1000.0;
    delta <= tolerance_ms
}

/// Space-hold push-to-talk detector.
#[derive(Debug, Clone)]
pub struct SpaceHold {
    space_run_inserted: usize,
    mechanical_run: u32,
    prev_space_gap: Option<Duration>,
    last_space_at: Option<Instant>,
    active: bool,
    release_deadline: Option<Instant>,
}

impl Default for SpaceHold {
    fn default() -> Self {
        Self::new()
    }
}

impl SpaceHold {
    /// Empty detector (no hold in progress).
    pub fn new() -> Self {
        Self {
            space_run_inserted: 0,
            mechanical_run: 0,
            prev_space_gap: None,
            last_space_at: None,
            active: false,
            release_deadline: None,
        }
    }

    /// True while a recognized hold is in progress.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Drive the state machine. `enabled` is omp `sttHoldEnabled` (typically
    /// `stt.enabled`); `autocomplete` matches `isShowingAutocomplete()`.
    pub fn handle(
        &mut self,
        canonical: &str,
        now: Instant,
        enabled: bool,
        autocomplete: bool,
    ) -> SpaceHoldOutcome {
        let is_space = canonical == "space";
        if self.active {
            if is_space {
                self.arm_release(now);
                return SpaceHoldOutcome::Swallow;
            }
            self.end();
            return SpaceHoldOutcome::EndThenContinue;
        }
        if !is_space {
            self.reset_run();
            return SpaceHoldOutcome::Continue;
        }
        if !enabled || autocomplete {
            return SpaceHoldOutcome::Continue;
        }
        let gap = self
            .last_space_at
            .map(|t| now.saturating_duration_since(t))
            .unwrap_or(Duration::MAX);
        let prev_gap = self.prev_space_gap;
        self.last_space_at = Some(now);
        self.prev_space_gap = Some(gap);
        if prev_gap.is_none() || !gaps_are_mechanical(gap, prev_gap.unwrap_or(Duration::MAX)) {
            self.mechanical_run = 0;
            self.space_run_inserted = self.space_run_inserted.saturating_add(1);
            return SpaceHoldOutcome::InsertSpace;
        }
        self.mechanical_run = self.mechanical_run.saturating_add(1);
        if self.mechanical_run >= SPACE_HOLD_MECHANICAL_RUN {
            let retract = self.space_run_inserted;
            self.reset_run();
            self.begin(now);
            return SpaceHoldOutcome::Start { retract };
        }
        SpaceHoldOutcome::Swallow
    }

    /// Fire `End` if the 250ms idle timeout has elapsed. Returns true when
    /// a hold just ended.
    pub fn poll(&mut self, now: Instant) -> bool {
        if !self.active {
            return false;
        }
        match self.release_deadline {
            Some(deadline) if now >= deadline => {
                self.end();
                true
            }
            _ => false,
        }
    }

    fn begin(&mut self, now: Instant) {
        self.active = true;
        self.arm_release(now);
    }

    fn arm_release(&mut self, now: Instant) {
        self.release_deadline = Some(now + Duration::from_millis(SPACE_HOLD_RELEASE_MS));
    }

    fn end(&mut self) {
        self.active = false;
        self.release_deadline = None;
        self.reset_run();
    }

    fn reset_run(&mut self) {
        self.space_run_inserted = 0;
        self.mechanical_run = 0;
        self.prev_space_gap = None;
        self.last_space_at = None;
    }
}

/// Delete up to `count` trailing characters (omp `deleteBeforeCursor`).
pub fn delete_before_cursor(buf: &mut String, count: usize) {
    let removable = count.min(buf.chars().count());
    if removable == 0 {
        return;
    }
    let keep = buf.chars().count() - removable;
    *buf = buf.chars().take(keep).collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn mechanical_gaps_are_steady_and_fast() {
        let a = Duration::from_millis(40);
        assert!(gaps_are_mechanical(a, a));
        assert!(gaps_are_mechanical(
            Duration::from_millis(40),
            Duration::from_millis(50)
        ));
        assert!(!gaps_are_mechanical(
            Duration::from_millis(40),
            Duration::from_millis(200)
        ));
        assert!(!gaps_are_mechanical(
            Duration::from_millis(40),
            Duration::from_millis(90)
        ));
    }

    #[test]
    fn single_tap_inserts_space() {
        let mut h = SpaceHold::new();
        let now = t0();
        assert_eq!(
            h.handle("space", now, true, false),
            SpaceHoldOutcome::InsertSpace
        );
        assert!(!h.is_active());
    }

    #[test]
    fn disabled_or_autocomplete_passes_through() {
        let mut h = SpaceHold::new();
        let now = t0();
        assert_eq!(
            h.handle("space", now, false, false),
            SpaceHoldOutcome::Continue
        );
        assert_eq!(
            h.handle("space", now, true, true),
            SpaceHoldOutcome::Continue
        );
    }

    #[test]
    fn hold_retracts_pre_burst_spaces() {
        let mut h = SpaceHold::new();
        let mut now = t0();
        // First space (no prev gap) → insert.
        assert_eq!(
            h.handle("space", now, true, false),
            SpaceHoldOutcome::InsertSpace
        );
        // Second space 40ms later: prev gap was huge → insert, reset run.
        now += Duration::from_millis(40);
        assert_eq!(
            h.handle("space", now, true, false),
            SpaceHoldOutcome::InsertSpace
        );
        // Third: mechanical vs 40ms, run=1 → swallow.
        now += Duration::from_millis(40);
        assert_eq!(
            h.handle("space", now, true, false),
            SpaceHoldOutcome::Swallow
        );
        // Fourth: run=2 → start, retract the two inserted spaces.
        now += Duration::from_millis(40);
        assert_eq!(
            h.handle("space", now, true, false),
            SpaceHoldOutcome::Start { retract: 2 }
        );
        assert!(h.is_active());
        // Further repeats are swallowed.
        now += Duration::from_millis(40);
        assert_eq!(
            h.handle("space", now, true, false),
            SpaceHoldOutcome::Swallow
        );
    }

    #[test]
    fn non_space_while_held_ends_then_continues() {
        let mut h = SpaceHold::new();
        let mut now = t0();
        let _ = h.handle("space", now, true, false);
        now += Duration::from_millis(40);
        let _ = h.handle("space", now, true, false);
        now += Duration::from_millis(40);
        let _ = h.handle("space", now, true, false);
        now += Duration::from_millis(40);
        let _ = h.handle("space", now, true, false);
        assert!(h.is_active());
        assert_eq!(
            h.handle("a", now, true, false),
            SpaceHoldOutcome::EndThenContinue
        );
        assert!(!h.is_active());
    }

    #[test]
    fn idle_timeout_ends_hold() {
        let mut h = SpaceHold::new();
        let mut now = t0();
        let _ = h.handle("space", now, true, false);
        now += Duration::from_millis(40);
        let _ = h.handle("space", now, true, false);
        now += Duration::from_millis(40);
        let _ = h.handle("space", now, true, false);
        now += Duration::from_millis(40);
        let _ = h.handle("space", now, true, false);
        assert!(h.is_active());
        now += Duration::from_millis(249);
        assert!(!h.poll(now));
        now += Duration::from_millis(2);
        assert!(h.poll(now));
        assert!(!h.is_active());
    }

    #[test]
    fn slow_taps_never_start_hold() {
        let mut h = SpaceHold::new();
        let mut now = t0();
        for _ in 0..6 {
            assert_eq!(
                h.handle("space", now, true, false),
                SpaceHoldOutcome::InsertSpace
            );
            now += Duration::from_millis(200);
        }
        assert!(!h.is_active());
    }

    #[test]
    fn delete_before_cursor_caps_at_len() {
        let mut s = "ab".to_owned();
        delete_before_cursor(&mut s, 5);
        assert!(s.is_empty());
        let mut s = "hello".to_owned();
        delete_before_cursor(&mut s, 2);
        assert_eq!(s, "hel");
    }
}
