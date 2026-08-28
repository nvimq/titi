//! Context compaction: a threshold policy, a chain of strategies with
//! fallback, and first-class append-only `CompactionEntry`s committed
//! into history (`docs/research/compaction-context`).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::session::entry::{new_id, now_ms};
use crate::session::Entry;

/// Errors surfaced by compaction.
#[derive(Debug)]
pub enum CompactionError {
    /// One strategy in the chain failed: which one and why.
    StrategyFailed { strategy: Strategy, reason: String },
    /// Every strategy in `method_order` failed; nothing was committed.
    Exhausted(Vec<CompactionError>),
}

impl fmt::Display for CompactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompactionError::StrategyFailed { strategy, reason } => {
                write!(f, "compaction strategy {}: {reason}", strategy.as_str())
            }
            CompactionError::Exhausted(failed) => {
                write!(f, "all compaction strategies failed: {}", failed.len())
            }
        }
    }
}

impl std::error::Error for CompactionError {}

/// Compaction strategies, tried in `method_order`. `Remote` is
/// provider-native (OpenAI-compatible compaction endpoints, arrives with
/// titi-providers); `SnapCompact` and `Handoff` are model-free; `Soft`
/// is the local LLM summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    Remote,
    SnapCompact,
    Handoff,
    Soft,
}

impl Strategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Strategy::Remote => "remote",
            Strategy::SnapCompact => "snapcompact",
            Strategy::Handoff => "handoff",
            Strategy::Soft => "soft",
        }
    }
}

/// First-class compaction record, appended to history. The messages from
/// `first_kept_entry_id` onward survive verbatim; everything before is
/// folded into `summary`. History is never rewritten — display
/// transcripts keep the full record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionEntry {
    pub id: String,
    pub ts: u64,
    pub strategy: Strategy,
    pub summary: String,
    pub short_summary: Option<String>,
    /// Id of the first message kept verbatim after this compaction.
    pub first_kept_entry_id: String,
    /// Estimated tokens across the messages at compaction time.
    pub tokens_before: u64,
}

impl CompactionEntry {
    pub fn new(
        strategy: Strategy,
        summary: impl Into<String>,
        first_kept_entry_id: impl Into<String>,
        tokens_before: u64,
    ) -> Self {
        Self {
            id: new_id(),
            ts: now_ms(),
            strategy,
            summary: summary.into(),
            short_summary: None,
            first_kept_entry_id: first_kept_entry_id.into(),
            tokens_before,
        }
    }
}

/// One node of the rebuildable context history: a verbatim message or a
/// committed compaction boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HistoryNode {
    Message(Entry),
    Compaction(CompactionEntry),
}

/// Where a compaction would cut: everything from `first_kept` (an index
/// into the evaluated messages) onward stays verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionTarget {
    pub first_kept: usize,
    pub total_tokens: u64,
}

/// Threshold and strategy-chain configuration.
#[derive(Debug, Clone)]
pub struct CompactionPolicy {
    /// Percent (0..=100) of the context window that arms compaction.
    pub threshold_percent: f64,
    /// Token budget for the verbatim tail; the newest message always
    /// survives even when it alone exceeds the budget.
    pub keep_recent_tokens: u64,
    /// Strategies tried in order; the first to succeed commits the entry.
    pub method_order: Vec<Strategy>,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            threshold_percent: 80.0,
            keep_recent_tokens: 20_000,
            method_order: vec![
                Strategy::Remote,
                Strategy::SnapCompact,
                Strategy::Handoff,
                Strategy::Soft,
            ],
        }
    }
}

impl CompactionPolicy {
    /// Whether `used_tokens` reached the threshold share of the window.
    /// At exactly the threshold the compaction fires.
    pub fn should_compact(&self, used_tokens: u64, context_window: u64) -> bool {
        context_window > 0
            && self.threshold_percent >= 0.0
            && used_tokens as f64 >= self.threshold_percent / 100.0 * context_window as f64
    }

    /// Computes the cut point of `messages` under `budget` tokens: the
    /// newest messages are kept whole while they fit; the rest is the
    /// prefix a strategy summarizes.
    pub fn compute_target(&self, messages: &[Entry], budget: u64) -> CompactionTarget {
        let total_tokens: u64 = messages.iter().map(|m| estimate_tokens(&m.content)).sum();
        let mut kept_tokens = 0u64;
        let mut first_kept = messages.len();
        for (i, m) in messages.iter().enumerate().rev() {
            let t = estimate_tokens(&m.content);
            if kept_tokens + t > budget {
                break;
            }
            kept_tokens += t;
            first_kept = i;
        }
        // Never cut an empty prefix: the newest message is always kept.
        if !messages.is_empty() {
            first_kept = first_kept.min(messages.len() - 1);
        }
        CompactionTarget { first_kept, total_tokens }
    }

    /// Runs the compaction chain over `history` when the threshold is
    /// reached. Only messages after the last committed compaction
    /// participate. On success the entry is appended — history is
    /// append-only, nothing is removed or rewritten. Returns `None` when
    /// the threshold is not reached or there is no prefix left to fold.
    pub fn compact<S: Summarizer>(
        &self,
        summarizer: &mut S,
        history: &mut Vec<HistoryNode>,
        used_tokens: u64,
        context_window: u64,
    ) -> Result<Option<CompactionEntry>, CompactionError> {
        if !self.should_compact(used_tokens, context_window) {
            return Ok(None);
        }
        let after_last = history
            .iter()
            .rposition(|n| matches!(n, HistoryNode::Compaction(_)))
            .map_or(0, |i| i + 1);
        let messages: Vec<Entry> = history[after_last..]
            .iter()
            .filter_map(|n| match n {
                HistoryNode::Message(m) => Some(m.clone()),
                HistoryNode::Compaction(_) => None,
            })
            .collect();
        let target = self.compute_target(&messages, self.keep_recent_tokens);
        if target.first_kept == 0 {
            return Ok(None);
        }
        let prefix = messages[..target.first_kept].to_vec();
        let mut failed = Vec::new();
        for strategy in &self.method_order {
            match summarizer.summarize(*strategy, &prefix) {
                Ok(summary) => {
                    let entry = CompactionEntry::new(
                        *strategy,
                        summary,
                        messages[target.first_kept].id.clone(),
                        target.total_tokens,
                    );
                    history.push(HistoryNode::Compaction(entry.clone()));
                    return Ok(Some(entry));
                }
                Err(e) => failed.push(e),
            }
        }
        Err(CompactionError::Exhausted(failed))
    }
}

/// Produces the summary for one strategy over the message prefix being
/// folded. Tests mock it; the real LLM-backed implementation arrives with
/// titi-providers.
pub trait Summarizer {
    fn summarize(
        &mut self,
        strategy: Strategy,
        messages: &[Entry],
    ) -> Result<String, CompactionError>;
}

/// Rough token estimate (~4 chars per token). Real counts come from
/// provider-reported usage once titi-providers lands.
pub fn estimate_tokens(text: &str) -> u64 {
    text.len().div_ceil(4) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Role;

    /// Mock summarizer: fails for the configured strategies, records the
    /// strategies it was asked about.
    struct MockSummarizer {
        fail: Vec<Strategy>,
        calls: Vec<Strategy>,
    }

    impl MockSummarizer {
        fn failing(fail: &[Strategy]) -> Self {
            Self { fail: fail.to_vec(), calls: Vec::new() }
        }
    }

    impl Summarizer for MockSummarizer {
        fn summarize(
            &mut self,
            strategy: Strategy,
            _messages: &[Entry],
        ) -> Result<String, CompactionError> {
            self.calls.push(strategy);
            if self.fail.contains(&strategy) {
                Err(CompactionError::StrategyFailed {
                    strategy,
                    reason: "unavailable".into(),
                })
            } else {
                Ok(format!("summary via {}", strategy.as_str()))
            }
        }
    }

    fn msg(len_chars: usize) -> Entry {
        Entry::new(None, Role::User, "x".repeat(len_chars))
    }

    /// Four messages of ~25 estimated tokens each.
    fn history() -> Vec<HistoryNode> {
        (0..4).map(|_| HistoryNode::Message(msg(100))).collect()
    }

    #[test]
    fn threshold_not_reached_compacts_nothing() {
        let mut s = MockSummarizer::failing(&[]);
        let mut h = history();
        let policy = CompactionPolicy {
            keep_recent_tokens: 30, // one ~25-token message fits
            ..CompactionPolicy::default()
        };

        // 50% of the window with an 80% threshold: no compaction.
        let out = policy
            .compact(&mut s, &mut h, 50_000, 100_000)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(out.is_none());
        assert!(s.calls.is_empty());
        assert_eq!(h.len(), 4);

        // At exactly the threshold it fires.
        let out = policy
            .compact(&mut s, &mut h, 80_000, 100_000)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(out.is_some());
    }

    #[test]
    fn chain_falls_through_strategies_in_order() {
        let policy = CompactionPolicy {
            keep_recent_tokens: 30, // one ~25-token message fits
            ..CompactionPolicy::default()
        };
        let mut s = MockSummarizer::failing(&[Strategy::Remote, Strategy::SnapCompact]);
        let mut h = history();

        let entry = policy
            .compact(&mut s, &mut h, 90_000, 100_000)
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("expected a compaction entry"));
        // The chain stopped at the first working strategy.
        assert_eq!(s.calls, vec![Strategy::Remote, Strategy::SnapCompact, Strategy::Handoff]);
        assert_eq!(entry.strategy, Strategy::Handoff);
        assert_eq!(entry.summary, "summary via handoff");
        // Boundary is the newest message: only it fit the tail budget.
        let kept_id = match h[3].clone() {
            HistoryNode::Message(m) => m.id,
            _ => panic!("expected a message"),
        };
        assert_eq!(entry.first_kept_entry_id, kept_id);
        assert_eq!(entry.tokens_before, 100);
    }

    #[test]
    fn entry_is_append_only_and_recompaction_starts_after_it() {
        let policy = CompactionPolicy {
            keep_recent_tokens: 30,
            ..CompactionPolicy::default()
        };
        let mut s = MockSummarizer::failing(&[]);
        let mut h = history();
        let before: Vec<HistoryNode> = h.clone();

        let entry = policy
            .compact(&mut s, &mut h, 90_000, 100_000)
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("expected a compaction entry"));

        // History grew by exactly one compaction node at the end; the
        // original messages are untouched.
        assert_eq!(h.len(), before.len() + 1);
        assert_eq!(h[..before.len()], before);
        assert_eq!(h.last(), Some(&HistoryNode::Compaction(entry.clone())));

        // A second compaction only sees messages after the entry — there
        // are none, so nothing fires even above threshold.
        s.calls.clear();
        let out = policy
            .compact(&mut s, &mut h, 90_000, 100_000)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(out.is_none());
        assert!(s.calls.is_empty());
        assert_eq!(h.len(), 5);
    }

    #[test]
    fn all_strategies_failing_is_an_error_and_commits_nothing() {
        let policy = CompactionPolicy {
            keep_recent_tokens: 30,
            ..CompactionPolicy::default()
        };
        let mut s = MockSummarizer::failing(&[
            Strategy::Remote,
            Strategy::SnapCompact,
            Strategy::Handoff,
            Strategy::Soft,
        ]);
        let mut h = history();

        let err = policy
            .compact(&mut s, &mut h, 90_000, 100_000)
            .expect_err("expected the chain to be exhausted");
        assert!(matches!(err, CompactionError::Exhausted(ref v) if v.len() == 4));
        assert_eq!(s.calls.len(), 4);
        // Nothing was committed.
        assert_eq!(h.len(), 4);
    }

    #[test]
    fn compute_target_keeps_tail_within_budget() {
        let policy = CompactionPolicy::default();
        let messages: Vec<Entry> = (0..4).map(|_| msg(100)).collect();

        // Budget for two messages (~50 tokens): keep the last two.
        let t = policy.compute_target(&messages, 50);
        assert_eq!(t.first_kept, 2);
        assert_eq!(t.total_tokens, 100);

        // Budget covers everything: nothing to fold.
        let t = policy.compute_target(&messages, 400);
        assert_eq!(t.first_kept, 0);

        // Budget smaller than one message: the newest is still kept.
        let t = policy.compute_target(&messages, 1);
        assert_eq!(t.first_kept, 3);

        let t = policy.compute_target(&[], 100);
        assert_eq!(t.first_kept, 0);
        assert_eq!(t.total_tokens, 0);
    }
}
