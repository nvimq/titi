//! Session checkpoints: rewind points recorded as a sidecar JSONL.
//!
//! The session file itself is never rewritten by taking a checkpoint, so the
//! append-only JSONL contract survives; checkpoints live beside it in
//! `<session_id>.checkpoints.jsonl`.

use serde::{Deserialize, Serialize};

/// A point a session can be rewound to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Leaf entry when the checkpoint was taken (`None` for an empty session).
    pub entry_id: Option<String>,
    /// Entries persisted when the checkpoint was taken.
    pub entries: usize,
    /// Milliseconds since the Unix epoch.
    pub ts: u64,
    /// Git commit the workspace was at when the checkpoint was taken.
    ///
    /// `None` for a session-only checkpoint, or when the workspace is not a git
    /// repository. Restoring it is what makes a rewind undo code, not only text.
    #[serde(default)]
    pub git_commit: Option<String>,
}
