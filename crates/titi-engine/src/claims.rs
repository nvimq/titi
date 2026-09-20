//! Per-file exclusive write claims.
//!
//! Two agents editing one file is the failure mode parallel dispatch creates,
//! so a write-tier tool call takes a claim for the duration of the call. Read
//! tiers are unaffected.
//!
//! Spec: `docs/research/empryo-port/README.md` (E4).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use smol_str::SmolStr;

/// Why a claim was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClaimError {
    /// Another agent already holds the file.
    #[error("{path} is claimed by {holder}")]
    Held { path: SmolStr, holder: SmolStr },
    /// The tool call carried no usable path.
    #[error("a write claim needs a workspace path")]
    NoPath,
}

#[derive(Default)]
struct Inner {
    holders: HashMap<SmolStr, SmolStr>,
}

/// A shared registry of per-file write claims.
///
/// Cheap to clone: every clone sees the same table, so a supervisor and the
/// tool loop that it guards share one view.
#[derive(Clone, Default)]
pub struct Claims {
    inner: Arc<Mutex<Inner>>,
}

impl Claims {
    pub fn new() -> Self {
        Self::default()
    }

    /// Canonical form of a workspace path, so `./src/a.rs`, `src//a.rs` and
    /// `src/a.rs/` are one claim.
    pub fn normalize(path: &str) -> SmolStr {
        let text = path.trim().replace('\\', "/");
        // Collapse separator runs, then drop a leading `./` and trailing `/`.
        let mut collapsed = String::with_capacity(text.len());
        for character in text.chars() {
            if character == '/' && collapsed.ends_with('/') {
                continue;
            }
            collapsed.push(character);
        }
        while let Some(rest) = collapsed.strip_prefix("./") {
            collapsed = rest.to_owned();
        }
        while collapsed.ends_with('/') {
            collapsed.pop();
        }
        SmolStr::from(collapsed)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A poisoned lock still holds a consistent table; carry on.
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Claims `path` for `agent_id`. Re-claiming your own file is a no-op, so
    /// a retry inside one agent never deadlocks itself.
    pub fn try_claim(&self, path: &str, agent_id: &str) -> Result<SmolStr, ClaimError> {
        let path = Self::normalize(path);
        if path.is_empty() {
            return Err(ClaimError::NoPath);
        }
        let holder: SmolStr = SmolStr::from(agent_id.trim());
        if holder.is_empty() {
            return Err(ClaimError::NoPath);
        }
        let mut inner = self.lock();
        match inner.holders.get(&path) {
            Some(current) if *current != holder => Err(ClaimError::Held {
                path,
                holder: current.clone(),
            }),
            _ => {
                inner.holders.insert(path.clone(), holder);
                Ok(path)
            }
        }
    }

    /// Releases `path` if `agent_id` holds it. Returns whether anything moved.
    pub fn release(&self, path: &str, agent_id: &str) -> bool {
        let path = Self::normalize(path);
        let mut inner = self.lock();
        match inner.holders.get(&path) {
            Some(current) if *current == agent_id.trim() => {
                inner.holders.remove(&path);
                true
            }
            _ => false,
        }
    }

    /// Who holds `path`, if anyone.
    pub fn holder(&self, path: &str) -> Option<SmolStr> {
        let path = Self::normalize(path);
        self.lock().holders.get(&path).cloned()
    }

    /// Drops every claim held by `agent_id`; returns how many were released.
    /// Called when an agent stops, so its files do not stay locked.
    pub fn release_all(&self, agent_id: &str) -> usize {
        let holder = agent_id.trim();
        let mut inner = self.lock();
        let before = inner.holders.len();
        inner.holders.retain(|_, current| current != holder);
        before - inner.holders.len()
    }

    /// Number of live claims.
    pub fn len(&self) -> usize {
        self.lock().holders.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_agent_cannot_take_a_held_file() {
        let claims = Claims::new();
        claims
            .try_claim("src/a.rs", "agent-1")
            .unwrap_or_else(|e| panic!("{e}"));
        let error = claims.try_claim("src/a.rs", "agent-2").unwrap_err();
        assert_eq!(
            error,
            ClaimError::Held {
                path: "src/a.rs".into(),
                holder: "agent-1".into()
            }
        );
        assert_eq!(claims.holder("src/a.rs").as_deref(), Some("agent-1"));
    }

    #[test]
    fn the_same_agent_may_reclaim_its_own_file() {
        let claims = Claims::new();
        claims
            .try_claim("a.rs", "agent-1")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(claims.try_claim("a.rs", "agent-1").is_ok());
        assert_eq!(claims.len(), 1);
    }

    #[test]
    fn paths_are_normalized_before_comparison() {
        let claims = Claims::new();
        claims
            .try_claim("./src/a.rs", "agent-1")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(claims.try_claim("src/a.rs", "agent-2").is_err());
        assert!(claims.try_claim("src//a.rs", "agent-2").is_err());
        assert!(claims.try_claim("src/a.rs/", "agent-2").is_err());
        assert_eq!(claims.len(), 1);
    }

    #[test]
    fn release_is_owner_only() {
        let claims = Claims::new();
        claims
            .try_claim("a.rs", "agent-1")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!claims.release("a.rs", "agent-2"), "not the holder");
        assert_eq!(claims.holder("a.rs").as_deref(), Some("agent-1"));
        assert!(claims.release("a.rs", "agent-1"));
        assert!(claims.is_empty());
    }

    #[test]
    fn release_all_frees_only_that_agents_files() {
        let claims = Claims::new();
        claims
            .try_claim("a.rs", "agent-1")
            .unwrap_or_else(|e| panic!("{e}"));
        claims
            .try_claim("b.rs", "agent-1")
            .unwrap_or_else(|e| panic!("{e}"));
        claims
            .try_claim("c.rs", "agent-2")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(claims.release_all("agent-1"), 2);
        assert_eq!(claims.len(), 1);
        assert_eq!(claims.holder("c.rs").as_deref(), Some("agent-2"));
    }

    #[test]
    fn an_empty_path_is_refused() {
        let claims = Claims::new();
        assert_eq!(claims.try_claim("   ", "agent-1"), Err(ClaimError::NoPath));
        assert_eq!(claims.try_claim("a.rs", ""), Err(ClaimError::NoPath));
        assert!(claims.is_empty());
    }
}
