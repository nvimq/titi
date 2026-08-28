//! One-shot fallback chain (Hermes-style): advance to the next provider only
//! on a turn boundary, only for retryable failures, and only once per chain.

use smol_str::SmolStr;

use crate::transport::{ApiKind, TransportError};

/// One candidate in the chain.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelRef {
    pub provider: SmolStr,
    pub model: SmolStr,
    pub api: ApiKind,
}

impl ModelRef {
    pub fn new(
        provider: impl Into<SmolStr>,
        model: impl Into<SmolStr>,
        api: ApiKind,
    ) -> Self {
        Self { provider: provider.into(), model: model.into(), api }
    }
}

/// Static fallback chain with one-shot activation semantics.
///
/// - `next()` is legal **only at a turn boundary** (between turns), never
///   mid-stream: switching after visible deltas would break replay of
///   thinking blocks.
/// - Only retryable failures (429, server errors, stalls) advance the chain.
/// - Once activated (the chain has handed out its fallback), it never cycles.
#[derive(Debug, Clone)]
pub struct FallbackChain {
    primary: ModelRef,
    backups: Vec<ModelRef>,
    /// Index into `backups` of the currently active entry; `None` while the
    /// primary is active.
    active: Option<usize>,
    activated: bool,
}

impl FallbackChain {
    pub fn new(primary: ModelRef, backups: Vec<ModelRef>) -> Self {
        Self { primary, backups, active: None, activated: false }
    }

    /// Currently selected entry (primary until activation).
    pub fn current(&self) -> &ModelRef {
        match self.active {
            None => &self.primary,
            Some(i) => &self.backups[i],
        }
    }

    pub fn is_activated(&self) -> bool {
        self.activated
    }

    /// Called at a turn boundary after a failed turn. Returns the next entry
    /// to try, or `None` when the chain is exhausted / the failure is fatal /
    /// the chain already activated.
    pub fn next(&mut self, err: &TransportError) -> Option<ModelRef> {
        if self.activated || !err.is_retryable() {
            return None;
        }
        let next_idx = match self.active {
            None if !self.backups.is_empty() => 0,
            Some(i) if i + 1 < self.backups.len() => i + 1,
            _ => return None,
        };
        self.activated = true;
        self.active = Some(next_idx);
        Some(self.backups[next_idx].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smol_str::SmolStr;

    fn chain() -> FallbackChain {
        FallbackChain::new(
            ModelRef::new("primary", "m1", ApiKind::OpenAiCompletions),
            vec![
                ModelRef::new("backup-a", "m2", ApiKind::AnthropicMessages),
                ModelRef::new("backup-b", "m3", ApiKind::OpenAiResponses),
            ],
        )
    }

    fn rate_limited() -> TransportError {
        TransportError::Retryable { status: Some(429), message: "rate limited".into() }
    }

    fn fatal() -> TransportError {
        TransportError::Fatal { status: Some(401), message: "bad key".into() }
    }

    #[test]
    fn rate_limit_advances_to_first_backup() {
        let mut c = chain();
        assert_eq!(c.current().provider, SmolStr::new("primary"));
        let next = c.next(&rate_limited()).expect("should advance");
        assert_eq!(next.provider, SmolStr::new("backup-a"));
        assert_eq!(c.current().provider, SmolStr::new("backup-a"));
        assert!(c.is_activated());
    }

    #[test]
    fn fatal_error_does_not_advance() {
        let mut c = chain();
        assert!(c.next(&fatal()).is_none());
        assert_eq!(c.current().provider, SmolStr::new("primary"));
        assert!(!c.is_activated());
    }

    #[test]
    fn one_shot_never_cycles() {
        let mut c = chain();
        assert!(c.next(&rate_limited()).is_some());
        // One-shot: even a retryable error never advances again.
        assert!(c.next(&rate_limited()).is_none());
        assert_eq!(c.current().provider, SmolStr::new("backup-a"));
    }

    #[test]
    fn exhausted_chain_returns_none() {
        let mut c = FallbackChain::new(
            ModelRef::new("p", "m", ApiKind::OpenAiCompletions),
            vec![ModelRef::new("b", "m", ApiKind::OpenAiCompletions)],
        );
        assert!(c.next(&rate_limited()).is_some());
        assert!(c.next(&rate_limited()).is_none());
    }

    #[test]
    fn empty_backups_never_advance() {
        let mut c = FallbackChain::new(
            ModelRef::new("p", "m", ApiKind::OpenAiCompletions),
            Vec::new(),
        );
        assert!(c.next(&rate_limited()).is_none());
        assert!(!c.is_activated());
    }

    #[test]
    fn stall_error_is_retryable() {
        let mut c = chain();
        let stalled = TransportError::Stalled { phase: crate::transport::StallPhase::Idle };
        assert!(c.next(&stalled).is_some());
    }

    #[test]
    fn mid_stream_refusal_is_caller_contract() {
        // The chain itself has no clock; the turn-boundary rule is that a
        // caller which saw deltas must NOT call `next()` — surfaced here by
        // documenting + exercising the fatal-after-content path: providers
        // translate mid-stream breakage into a non-retryable turn error.
        let mut c = chain();
        let mid_stream = TransportError::Fatal { status: None, message: "stream broke after deltas".into() };
        assert!(c.next(&mid_stream).is_none());
        assert_eq!(c.current().provider, SmolStr::new("primary"));
    }
}
