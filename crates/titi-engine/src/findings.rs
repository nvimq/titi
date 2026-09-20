//! Ordered, bounded log of subagent findings.
//!
//! A parent collects what its subagents learned without polling each one:
//! every finding gets a monotonic sequence number and the reader keeps a
//! cursor. The log is bounded, so a long run cannot grow it without limit.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use smol_str::SmolStr;

/// How many findings are kept before the oldest are dropped.
pub const FINDINGS_CAPACITY: usize = 256;

/// One thing an agent learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Monotonic position in the log; never reused.
    pub seq: u64,
    pub agent_id: SmolStr,
    /// Turn that produced it, when the finding came from a turn.
    pub turn_id: Option<u64>,
    pub text: SmolStr,
}

#[derive(Default)]
struct Inner {
    log: VecDeque<Finding>,
    next_seq: u64,
    dropped: u64,
}

#[derive(Clone)]
pub struct Findings {
    inner: Arc<Mutex<Inner>>,
    capacity: usize,
}

impl Default for Findings {
    fn default() -> Self {
        Self::new(FINDINGS_CAPACITY)
    }
}

impl Findings {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            capacity: capacity.max(1),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Records a finding and returns its sequence number.
    pub fn push(
        &self,
        agent_id: impl Into<SmolStr>,
        turn_id: Option<u64>,
        text: impl Into<SmolStr>,
    ) -> u64 {
        let mut inner = self.lock();
        let seq = inner.next_seq;
        inner.next_seq += 1;
        inner.log.push_back(Finding {
            seq,
            agent_id: agent_id.into(),
            turn_id,
            text: text.into(),
        });
        while inner.log.len() > self.capacity {
            inner.log.pop_front();
            inner.dropped += 1;
        }
        seq
    }

    /// Findings recorded after `cursor`, plus the cursor to pass next time.
    /// A cursor older than the retained window simply yields what is left.
    pub fn drain_since(&self, cursor: u64) -> (u64, Vec<Finding>) {
        let inner = self.lock();
        let next = inner.next_seq;
        let findings = inner
            .log
            .iter()
            .filter(|finding| finding.seq >= cursor)
            .cloned()
            .collect();
        (next, findings)
    }

    /// Cursor to start from, so a reader sees only findings recorded later.
    pub fn cursor(&self) -> u64 {
        self.lock().next_seq
    }

    /// Findings currently retained.
    pub fn len(&self) -> usize {
        self.lock().log.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many findings fell off the front of the bounded log.
    pub fn dropped(&self) -> u64 {
        self.lock().dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_since_returns_only_new_findings() {
        let findings = Findings::new(8);
        let start = findings.cursor();
        findings.push("agent-1", Some(1), "found the bug");
        findings.push("agent-2", None, "checked the tests");

        let (cursor, batch) = findings.drain_since(start);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].text, "found the bug");
        assert_eq!(batch[0].turn_id, Some(1));
        assert_eq!(batch[1].agent_id, "agent-2");

        // Nothing new: an empty batch and a stable cursor.
        let (again, empty) = findings.drain_since(cursor);
        assert!(empty.is_empty());
        assert_eq!(again, cursor);

        findings.push("agent-1", None, "and the fix");
        let (_, batch) = findings.drain_since(cursor);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].text, "and the fix");
    }

    #[test]
    fn sequence_numbers_are_monotonic_and_never_reused() {
        let findings = Findings::new(2);
        let first = findings.push("a", None, "one");
        let second = findings.push("a", None, "two");
        assert!(second > first);
        // Overflow drops the oldest, but the counter keeps climbing.
        findings.push("a", None, "three");
        assert_eq!(findings.len(), 2);
        assert_eq!(findings.dropped(), 1);
        let (cursor, batch) = findings.drain_since(0);
        assert_eq!(cursor, 3);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].seq, 1, "the oldest retained finding");
    }

    #[test]
    fn a_stale_cursor_yields_what_is_left() {
        let findings = Findings::new(1);
        findings.push("a", None, "old");
        findings.push("a", None, "new");
        let (_, batch) = findings.drain_since(0);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].text, "new");
    }
}
