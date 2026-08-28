//! SQLite + FTS5 index over sessions — a derivable structure built
//! incrementally on every append (`docs/research/sessions-persistence`).

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use super::entry::Entry;
use super::{SessionError, SessionMeta};

/// A full-text search hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub session_id: String,
    pub entry_id: String,
    pub text: String,
}

/// SQLite index at `<agent_dir>/state.db` (WAL): session catalog, entries,
/// and an FTS5 table over entry text.
pub struct SessionIndex {
    conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    id         TEXT PRIMARY KEY,
    title      TEXT,
    created_at INTEGER NOT NULL,
    bot_id     TEXT,
    source     TEXT
);
CREATE TABLE IF NOT EXISTS entries (
    session_id TEXT NOT NULL,
    entry_id   TEXT NOT NULL,
    ts         INTEGER NOT NULL,
    text       TEXT NOT NULL,
    PRIMARY KEY (session_id, entry_id)
);
CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
    text, entry_id UNINDEXED, session_id UNINDEXED
);
";

impl SessionIndex {
    /// Opens (or creates) the index database, running the schema migration.
    pub fn open(path: &Path) -> Result<Self, SessionError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(SessionError::Io)?;
        }
        let conn = Connection::open(path).map_err(SessionError::Db)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;").map_err(SessionError::Db)?;
        conn.execute_batch(SCHEMA).map_err(SessionError::Db)?;
        Ok(Self { conn })
    }

    /// Records a session in the catalog.
    pub fn insert_session(
        &self,
        id: &str,
        created_at: u64,
        meta: &SessionMeta,
    ) -> Result<(), SessionError> {
        self.conn
            .execute(
                "INSERT INTO sessions (id, title, created_at, bot_id, source)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, meta.title, created_at as i64, meta.bot_id, meta.source],
            )
            .map_err(SessionError::Db)?;
        Ok(())
    }

    /// Indexes one entry: catalog row + FTS row.
    pub fn index_entry(&self, session_id: &str, entry: &Entry) -> Result<(), SessionError> {
        self.conn
            .execute(
                "INSERT INTO entries (session_id, entry_id, ts, text) VALUES (?1, ?2, ?3, ?4)",
                params![session_id, entry.id, entry.ts as i64, entry.content],
            )
            .map_err(SessionError::Db)?;
        self.conn
            .execute(
                "INSERT INTO entries_fts (text, entry_id, session_id) VALUES (?1, ?2, ?3)",
                params![entry.content, entry.id, session_id],
            )
            .map_err(SessionError::Db)?;
        Ok(())
    }

    /// Full-text search over indexed entries, optionally isolated to one bot.
    ///
    /// The raw query is wrapped as an FTS5 phrase, so user input can never
    /// alter the query grammar.
    pub fn search(
        &self,
        query: &str,
        bot_id: Option<&str>,
    ) -> Result<Vec<SearchHit>, SessionError> {
        let phrase = format!("\"{}\"", query.replace('"', "\"\""));
        let mut stmt = self
            .conn
            .prepare(
                "SELECT entries_fts.entry_id, entries_fts.session_id, entries_fts.text
                 FROM entries_fts JOIN sessions ON sessions.id = entries_fts.session_id
                 WHERE entries_fts MATCH ?1 AND (?2 IS NULL OR sessions.bot_id = ?2)
                 ORDER BY rank",
            )
            .map_err(SessionError::Db)?;
        let hits = stmt
            .query_map(params![phrase, bot_id], |row| {
                Ok(SearchHit {
                    entry_id: row.get(0)?,
                    session_id: row.get(1)?,
                    text: row.get(2)?,
                })
            })
            .map_err(SessionError::Db)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(SessionError::Db)?;
        Ok(hits)
    }

    /// Id of the most recently created session, if any.
    pub fn resume_latest(&self) -> Result<Option<String>, SessionError> {
        self.conn
            .query_row(
                "SELECT id FROM sessions ORDER BY created_at DESC, rowid DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(SessionError::Db)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_index() -> (tempfile::TempDir, SessionIndex) {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
        let index = SessionIndex::open(&dir.path().join("state.db"))
            .unwrap_or_else(|e| panic!("open: {e}"));
        (dir, index)
    }

    fn meta(bot_id: Option<&str>) -> SessionMeta {
        SessionMeta {
            title: None,
            bot_id: bot_id.map(String::from),
            source: Some("cli".into()),
        }
    }

    #[test]
    fn resume_latest_returns_newest_session() {
        let (_dir, index) = tmp_index();
        assert_eq!(index.resume_latest().unwrap_or_else(|e| panic!("{e}")), None);
        index.insert_session("s1", 100, &meta(None)).unwrap_or_else(|e| panic!("{e}"));
        index.insert_session("s2", 200, &meta(None)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(index.resume_latest().unwrap_or_else(|e| panic!("{e}")), Some("s2".into()));
    }

    #[test]
    fn search_matches_phrase_and_prefix_tokens() {
        let (_dir, index) = tmp_index();
        let e = Entry::new(None, super::super::Role::User, "deploy kafka cluster");
        index.insert_session("s1", 1, &meta(None)).unwrap_or_else(|e| panic!("{e}"));
        index.index_entry("s1", &e).unwrap_or_else(|e| panic!("{e}"));

        let hits = index.search("kafka", None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry_id, e.id);

        // Phrase matches only the exact token sequence, not arbitrary text.
        assert!(index.search("kafka deploy", None).unwrap_or_else(|e| panic!("{e}")).is_empty());
        assert!(index.search("deploy kafka", None).unwrap_or_else(|e| panic!("{e}")).len() == 1);
    }

    #[test]
    fn fts_query_injection_is_neutralized() {
        let (_dir, index) = tmp_index();
        let e = Entry::new(None, super::super::Role::User, "safe text");
        index.insert_session("s1", 1, &meta(None)).unwrap_or_else(|e| panic!("{e}"));
        index.index_entry("s1", &e).unwrap_or_else(|e| panic!("{e}"));
        // Raw FTS5 grammar in user input must not error or escape the phrase.
        assert!(index
            .search("safe\" OR (1=1) AND \"", None)
            .unwrap_or_else(|e| panic!("{e}"))
            .is_empty());
    }
}
