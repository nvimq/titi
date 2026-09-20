//! JSONL persistence for append-only session trees.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use super::entry::{self, Entry, Role};
use super::index::{SearchHit, SessionIndex};
use super::{SessionError, SessionMeta};

/// Filesystem store: one JSONL file per session under `<agent_dir>/sessions`,
/// a per-session leaf pointer, and the SQLite/FTS5 index at
/// `<agent_dir>/state.db`. The agent directory is injected — no env lookups —
/// so tests point it at a `tempfile::TempDir`.
pub struct SessionStore {
    dir: PathBuf,
    index: SessionIndex,
}

impl SessionStore {
    /// Opens the store rooted at `<agent_dir>/sessions`.
    pub fn new(agent_dir: &Path) -> Result<Self, SessionError> {
        let dir = agent_dir.join("sessions");
        fs::create_dir_all(&dir).map_err(SessionError::Io)?;
        let index = SessionIndex::open(&agent_dir.join("state.db"))?;
        Ok(Self { dir, index })
    }

    /// Creates an empty session and returns its id.
    pub fn create(&self, meta: SessionMeta) -> Result<String, SessionError> {
        let id = entry::new_id();
        File::create_new(self.session_file(&id)).map_err(|e| SessionError::Io(e))?;
        self.index.insert_session(&id, entry::now_ms(), &meta)?;
        Ok(id)
    }

    /// Loads every parseable entry of a session, oldest first.
    ///
    /// Lenient by design: a torn final line left by a crash is skipped.
    pub fn open(&self, session_id: &str) -> Result<Vec<Entry>, SessionError> {
        self.load(session_id)
    }

    /// Appends an entry as a child of the current leaf; the leaf moves to it.
    pub fn append(
        &self,
        session_id: &str,
        role: Role,
        content: &str,
    ) -> Result<Entry, SessionError> {
        if !self.session_file(session_id).exists() {
            return Err(SessionError::NotFound(session_id.into()));
        }
        let parent = self.current_leaf(session_id)?;
        let e = Entry::new(parent, role, content);
        let mut file = OpenOptions::new()
            .append(true)
            .open(self.session_file(session_id))
            .map_err(SessionError::Io)?;
        let line = serde_json::to_string(&e).map_err(SessionError::Json)?;
        writeln!(file, "{line}").map_err(SessionError::Io)?;
        self.set_leaf(session_id, &e.id)?;
        self.index.index_entry(session_id, &e)?;
        Ok(e)
    }

    /// Returns one entry by id.
    pub fn entry(&self, session_id: &str, entry_id: &str) -> Result<Option<Entry>, SessionError> {
        Ok(self
            .load(session_id)?
            .into_iter()
            .find(|e| e.id == entry_id))
    }

    /// Moves the leaf pointer to `from_entry_id` without touching history.
    /// The next [`append`](Self::append) becomes a child of that entry.
    pub fn fork(&self, session_id: &str, from_entry_id: &str) -> Result<(), SessionError> {
        if self.entry(session_id, from_entry_id)?.is_none() {
            return Err(SessionError::NotFound(format!(
                "{session_id}/{from_entry_id}"
            )));
        }
        self.set_leaf(session_id, from_entry_id)
    }

    /// Path from the root to `leaf` (or the current leaf when `None`),
    /// oldest first. Cycle-safe: each id is visited at most once.
    pub fn walk(&self, session_id: &str, leaf: Option<&str>) -> Result<Vec<Entry>, SessionError> {
        let entries = self.load(session_id)?;
        let by_id: HashMap<&str, &Entry> = entries.iter().map(|e| (e.id.as_str(), e)).collect();
        let mut cur = match leaf {
            Some(id) => Some(id.to_string()),
            None => self.current_leaf(session_id)?,
        };
        let mut chain = Vec::new();
        let mut visited = HashSet::new();
        while let Some(id) = cur {
            if !visited.insert(id.clone()) {
                break;
            }
            match by_id.get(id.as_str()) {
                Some(e) => {
                    chain.push((*e).clone());
                    cur = e.parent_id.clone();
                }
                None => break,
            }
        }
        chain.reverse();
        Ok(chain)
    }

    /// Id of the most recently created session, if any.
    pub fn resume_latest(&self) -> Result<Option<String>, SessionError> {
        self.index.resume_latest()
    }

    /// Full-text search over indexed entries, optionally scoped to one bot.
    pub fn search(
        &self,
        query: &str,
        bot_id: Option<&str>,
    ) -> Result<Vec<SearchHit>, SessionError> {
        self.index.search(query, bot_id)
    }

    fn session_file(&self, session_id: &str) -> PathBuf {
        self.dir.join(format!("{session_id}.jsonl"))
    }

    fn leaf_file(&self, session_id: &str) -> PathBuf {
        self.dir.join(format!("{session_id}.leaf"))
    }

    fn current_leaf(&self, session_id: &str) -> Result<Option<String>, SessionError> {
        match fs::read_to_string(self.leaf_file(session_id)) {
            Ok(s) => {
                let t = s.trim();
                Ok((!t.is_empty()).then(|| t.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(SessionError::Io(e)),
        }
    }

    fn set_leaf(&self, session_id: &str, entry_id: &str) -> Result<(), SessionError> {
        fs::write(self.leaf_file(session_id), entry_id).map_err(SessionError::Io)
    }

    fn load(&self, session_id: &str) -> Result<Vec<Entry>, SessionError> {
        let file = self.session_file(session_id);
        if !file.exists() {
            return Err(SessionError::NotFound(session_id.into()));
        }
        let f = File::open(&file).map_err(SessionError::Io)?;
        let mut out = Vec::new();
        for line in BufReader::new(f).lines() {
            let line = line.map_err(SessionError::Io)?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // Lenient: a torn final line after a crash is skipped, not fatal.
            if let Ok(e) = serde_json::from_str::<Entry>(line) {
                out.push(e);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
        let store = SessionStore::new(dir.path()).unwrap_or_else(|e| panic!("new: {e}"));
        (dir, store)
    }

    fn meta(bot_id: &str) -> SessionMeta {
        SessionMeta {
            title: Some(format!("bot-{bot_id}")),
            bot_id: Some(bot_id.into()),
            source: Some("cli".into()),
        }
    }

    #[test]
    fn jsonl_roundtrip_preserves_entries() {
        let (_dir, s) = store();
        let sid = s.create(meta("a")).unwrap_or_else(|e| panic!("{e}"));
        let a = s
            .append(&sid, Role::User, "hello")
            .unwrap_or_else(|e| panic!("{e}"));
        let b = s
            .append(&sid, Role::Assistant, "world")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(b.parent_id.as_deref(), Some(a.id.as_str()));

        // Raw file: one JSON object per line, in append order.
        let raw = fs::read_to_string(s.session_file(&sid)).unwrap_or_else(|e| panic!("{e}"));
        let parsed: Vec<Entry> = raw
            .lines()
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("line: {e}")))
            .collect();
        assert_eq!(parsed, vec![a.clone(), b.clone()]);

        // Reopening the store (fresh index connection) replays the same tree.
        let reopened = SessionStore::new(_dir.path()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            reopened.open(&sid).unwrap_or_else(|e| panic!("{e}")),
            vec![a, b]
        );
    }

    #[test]
    fn fork_moves_leaf_without_rewriting_history() {
        let (_dir, s) = store();
        let sid = s.create(meta("a")).unwrap_or_else(|e| panic!("{e}"));
        let a = s
            .append(&sid, Role::User, "a")
            .unwrap_or_else(|e| panic!("{e}"));
        let b = s
            .append(&sid, Role::Assistant, "b")
            .unwrap_or_else(|e| panic!("{e}"));

        s.fork(&sid, &a.id).unwrap_or_else(|e| panic!("{e}"));
        let c = s
            .append(&sid, Role::User, "c")
            .unwrap_or_else(|e| panic!("{e}"));

        // New append is a child of the fork target, not of the old leaf.
        assert_eq!(c.parent_id.as_deref(), Some(a.id.as_str()));
        // Parent entries are untouched on disk.
        let all = s.open(&sid).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(all, vec![a.clone(), b.clone(), c]);
        assert_eq!(all[1].parent_id.as_deref(), Some(a.id.as_str()));
    }

    #[test]
    fn walk_after_fork_follows_new_leaf() {
        let (_dir, s) = store();
        let sid = s.create(meta("a")).unwrap_or_else(|e| panic!("{e}"));
        let a = s
            .append(&sid, Role::User, "a")
            .unwrap_or_else(|e| panic!("{e}"));
        let b = s
            .append(&sid, Role::Assistant, "b")
            .unwrap_or_else(|e| panic!("{e}"));

        // Before fork: full path to current leaf.
        assert_eq!(
            s.walk(&sid, None).unwrap_or_else(|e| panic!("{e}")),
            vec![a.clone(), b.clone()]
        );

        s.fork(&sid, &a.id).unwrap_or_else(|e| panic!("{e}"));
        let c = s
            .append(&sid, Role::User, "c")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            s.walk(&sid, None).unwrap_or_else(|e| panic!("{e}")),
            vec![a.clone(), c.clone()]
        );
        // Explicit leaf still works, and b remains reachable.
        assert_eq!(
            s.walk(&sid, Some(&b.id)).unwrap_or_else(|e| panic!("{e}")),
            vec![a.clone(), b]
        );
        assert_eq!(
            s.walk(&sid, Some(&c.id)).unwrap_or_else(|e| panic!("{e}")),
            vec![a, c]
        );
    }

    #[test]
    fn fork_to_missing_entry_is_not_found() {
        let (_dir, s) = store();
        let sid = s.create(meta("a")).unwrap_or_else(|e| panic!("{e}"));
        let err = s.fork(&sid, "nope").unwrap_err();
        assert!(matches!(err, SessionError::NotFound(_)));
    }

    #[test]
    fn fts_finds_appended_content() {
        let (_dir, s) = store();
        let sid = s.create(meta("a")).unwrap_or_else(|e| panic!("{e}"));
        let e = s
            .append(&sid, Role::User, "deploy kafka cluster")
            .unwrap_or_else(|e| panic!("{e}"));
        let hits = s.search("kafka", None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry_id, e.id);
        assert_eq!(hits[0].session_id, sid);
    }

    #[test]
    fn bot_id_isolation_a_cannot_see_b() {
        let (_dir, s) = store();
        let sa = s.create(meta("bot-a")).unwrap_or_else(|e| panic!("{e}"));
        let sb = s.create(meta("bot-b")).unwrap_or_else(|e| panic!("{e}"));
        let ea = s
            .append(&sa, Role::User, "alpha secret plan")
            .unwrap_or_else(|e| panic!("{e}"));
        let eb = s
            .append(&sb, Role::User, "beta secret plan")
            .unwrap_or_else(|e| panic!("{e}"));

        let for_a = s
            .search("secret", Some("bot-a"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(for_a.len(), 1);
        assert_eq!(for_a[0].entry_id, ea.id);
        assert_ne!(for_a[0].session_id, sb);

        let for_b = s
            .search("secret", Some("bot-b"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(for_b.len(), 1);
        assert_eq!(for_b[0].entry_id, eb.id);

        assert_eq!(
            s.search("secret", None)
                .unwrap_or_else(|e| panic!("{e}"))
                .len(),
            2
        );
    }

    #[test]
    fn resume_latest_returns_last_created_session() {
        let (_dir, s) = store();
        assert_eq!(s.resume_latest().unwrap_or_else(|e| panic!("{e}")), None);
        let first = s.create(meta("a")).unwrap_or_else(|e| panic!("{e}"));
        let second = s.create(meta("a")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            s.resume_latest().unwrap_or_else(|e| panic!("{e}")),
            Some(second.clone())
        );
        assert_ne!(first, second);
    }

    #[test]
    fn torn_final_line_is_skipped_leniently() {
        let (_dir, s) = store();
        let sid = s.create(meta("a")).unwrap_or_else(|e| panic!("{e}"));
        let a = s
            .append(&sid, Role::User, "intact")
            .unwrap_or_else(|e| panic!("{e}"));
        // Simulate kill -9 mid-write: append a torn line without newline.
        {
            let mut f = OpenOptions::new()
                .append(true)
                .open(s.session_file(&sid))
                .unwrap_or_else(|e| panic!("{e}"));
            write!(f, "{{\"id\":\"torn").unwrap_or_else(|e| panic!("{e}"));
        }
        let all = s.open(&sid).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(all, vec![a.clone()]);
        // The tree stays usable: append after recovery still works.
        let b = s
            .append(&sid, Role::Assistant, "after crash")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(b.parent_id.as_deref(), Some(a.id.as_str()));
    }

    #[test]
    fn unknown_session_errors_on_append_and_open() {
        let (_dir, s) = store();
        assert!(matches!(
            s.append("ghost", Role::User, "x"),
            Err(SessionError::NotFound(_))
        ));
        assert!(matches!(s.open("ghost"), Err(SessionError::NotFound(_))));
    }
}
