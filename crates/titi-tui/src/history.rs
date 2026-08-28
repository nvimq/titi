//! History batch + ack handshake (contract: `omp://tui-core-renderer`,
//! spec: `docs/research/tui-renderer/frame-history.md`).
//!
//! The renderer accepts each batch exactly once, acknowledges strictly
//! monotonic ids, and rejects repeats / non-monotonic ids. A `Replay` batch
//! replaces the whole logical ledger atomically.

/// Kind of a history batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchKind {
    /// Finalized rows appended to the existing history.
    Append,
    /// Full logical ledger, replaces history atomically (resize/reset paths).
    Replay,
}

/// History offered by the frame provider; finality is the application's
/// decision, never an inference from a row crossing the top of the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryBatch {
    pub id: u64,
    pub rows: Vec<String>,
    pub kind: BatchKind,
}

/// Acknowledgment of a written batch; sent back to the provider only after
/// the batch has been accepted (written) exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ack {
    pub id: u64,
}

/// Why a batch was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    /// Id equal to the last accepted one: already written.
    Duplicate,
    /// Id lower than the last accepted one: out of order.
    NonMonotonic,
}

/// Engine-side history state: ack cursor plus the accumulated ledger.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HistoryState {
    last_acked: Option<u64>,
    rows: Vec<String>,
}

impl HistoryState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Id of the last accepted batch, if any.
    pub fn last_acked(&self) -> Option<u64> {
        self.last_acked
    }

    /// Accumulated history rows (append-extended or replay-replaced).
    pub fn rows(&self) -> &[String] {
        &self.rows
    }
}

/// Handshake: accept `batch` into `state` and return its [`Ack`], or reject it.
///
/// A batch is accepted exactly once — its id must be strictly greater than the
/// last accepted id. Rejection leaves `state` untouched (replay stays atomic:
/// either the whole ledger is replaced or nothing changes).
pub fn accept_batch(state: &mut HistoryState, batch: HistoryBatch) -> Result<Ack, Reject> {
    match state.last_acked {
        Some(last) if batch.id == last => return Err(Reject::Duplicate),
        Some(last) if batch.id < last => return Err(Reject::NonMonotonic),
        _ => {}
    }
    match batch.kind {
        BatchKind::Append => state.rows.extend(batch.rows),
        BatchKind::Replay => state.rows = batch.rows,
    }
    state.last_acked = Some(batch.id);
    Ok(Ack { id: batch.id })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(id: u64, rows: &[&str], kind: BatchKind) -> HistoryBatch {
        HistoryBatch {
            id,
            rows: rows.iter().map(|s| (*s).to_owned()).collect(),
            kind,
        }
    }

    #[test]
    fn monotonic_ids_are_accepted() {
        let mut state = HistoryState::new();
        for id in 1..=3u64 {
            let ack = accept_batch(&mut state, batch(id, &["row"], BatchKind::Append)).unwrap();
            assert_eq!(ack, Ack { id });
            assert_eq!(state.last_acked(), Some(id));
        }
        assert_eq!(state.rows().len(), 3);
    }

    #[test]
    fn lower_id_is_rejected() {
        let mut state = HistoryState::new();
        accept_batch(&mut state, batch(3, &["a"], BatchKind::Append)).unwrap();
        let before = state.clone();
        assert_eq!(
            accept_batch(&mut state, batch(1, &["b"], BatchKind::Append)),
            Err(Reject::NonMonotonic)
        );
        assert_eq!(state, before, "rejected batch must not mutate state");
    }

    #[test]
    fn repeated_id_is_rejected() {
        let mut state = HistoryState::new();
        accept_batch(&mut state, batch(3, &["a"], BatchKind::Append)).unwrap();
        let before = state.clone();
        assert_eq!(
            accept_batch(&mut state, batch(3, &["a"], BatchKind::Append)),
            Err(Reject::Duplicate)
        );
        assert_eq!(state, before);
    }

    #[test]
    fn replay_replaces_ledger_atomically() {
        let mut state = HistoryState::new();
        accept_batch(&mut state, batch(1, &["old1", "old2"], BatchKind::Append)).unwrap();
        let ack = accept_batch(&mut state, batch(2, &["new ledger"], BatchKind::Replay)).unwrap();
        assert_eq!(ack, Ack { id: 2 });
        assert_eq!(state.rows(), ["new ledger"]);
    }

    #[test]
    fn rejected_replay_leaves_ledger_untouched() {
        let mut state = HistoryState::new();
        accept_batch(&mut state, batch(5, &["keep"], BatchKind::Append)).unwrap();
        assert_eq!(
            accept_batch(&mut state, batch(4, &["stale"], BatchKind::Replay)),
            Err(Reject::NonMonotonic)
        );
        assert_eq!(state.rows(), ["keep"]);
        assert_eq!(state.last_acked(), Some(5));
    }
}
