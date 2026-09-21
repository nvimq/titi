//! A searchable memory index, beside the markdown snapshot.
//!
//! The markdown stores are injected whole, so they are capped small. This
//! index is not: a memory is a row, deduplicated by a hash of its text, tied
//! to the files it is about, and found by FTS plus a local embedding.
//!
//! The embedding is a hashed bag of character trigrams, not a model. It needs
//! no download and no network, and it is stable: the same text always lands
//! on the same vector, so similarity survives a restart. It is worse than a
//! trained model at paraphrase and that is the trade.

use std::path::Path;

use rusqlite::{Connection, params};

use crate::embed::Embedder;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// How many dimensions the bag-of-trigrams vector has.
pub const EMBED_DIM: usize = 256;

/// Cosine above this counts as the same fact said twice.
const DEDUP_COSINE: f32 = 0.9;

/// How many memories a recall returns.
pub const RECALL_LIMIT: usize = 5;

/// One stored memory.
#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    pub id: i64,
    pub category: String,
    pub summary: String,
    pub details: String,
    pub use_count: i64,
}

/// Why a write did not insert a new row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remembered {
    /// A new row.
    Added(i64),
    /// The same text was already stored; its counter went up.
    Duplicate(i64),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("memory db: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("memory io: {0}")]
    Io(#[from] std::io::Error),
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS memories (
    id         INTEGER PRIMARY KEY,
    category   TEXT NOT NULL DEFAULT 'context',
    summary    TEXT NOT NULL,
    details    TEXT NOT NULL DEFAULT '',
    hash       TEXT NOT NULL UNIQUE,
    embedding  BLOB NOT NULL,
    use_count  INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE IF NOT EXISTS memory_files (
    memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    path      TEXT NOT NULL,
    PRIMARY KEY (memory_id, path)
);
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    summary, details, content='memories', content_rowid='id'
);
";

/// The index at `<agent_dir>/memory.db`.
pub struct MemoryIndex {
    conn: Connection,
}

impl MemoryIndex {
    /// Opens or creates the index.
    pub fn open(agent_dir: &Path) -> Result<Self, Error> {
        std::fs::create_dir_all(agent_dir)?;
        let conn = Connection::open(agent_dir.join("memory.db"))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;
        // Columns added after the first release. CREATE IF NOT EXISTS does not
        // alter a table that already exists.
        for (name, ddl) in [
            ("embedder", "TEXT NOT NULL DEFAULT 'local-trigram-v1'"),
            ("pinned", "INTEGER NOT NULL DEFAULT 0"),
            ("hidden", "INTEGER NOT NULL DEFAULT 0"),
        ] {
            ensure_column(&conn, name, ddl)?;
        }
        Ok(Self { conn })
    }

    /// Stores a memory, or bumps the counter when the same text exists.
    ///
    /// `paths` are the files the memory is about, so a later recall can prefer
    /// it while those files are being edited.
    pub fn remember(
        &self,
        category: &str,
        summary: &str,
        details: &str,
        paths: &[String],
    ) -> Result<Remembered, Error> {
        let hash = content_hash(summary, details);
        if let Some(id) = self.id_for_hash(&hash)? {
            self.conn.execute(
                "UPDATE memories SET use_count = use_count + 1 WHERE id = ?1",
                params![id],
            )?;
            self.link(id, paths)?;
            return Ok(Remembered::Duplicate(id));
        }
        let embedder = crate::embed::LocalEmbedder;
        let vector = embedder.embed(&format!("{summary}\n{details}"));
        self.conn.execute(
            "INSERT INTO memories (category, summary, details, hash, embedding, embedder)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                category,
                summary.trim(),
                details.trim(),
                hash,
                bytemuck_vec(&vector),
                embedder.name(),
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.conn.execute(
            "INSERT INTO memories_fts (rowid, summary, details) VALUES (?1, ?2, ?3)",
            params![id, summary.trim(), details.trim()],
        )?;
        self.link(id, paths)?;
        Ok(Remembered::Added(id))
    }

    /// The memories most relevant to `query` and the files being touched.
    ///
    /// FTS finds candidates; the embedding reranks them; a memory linked to a
    /// touched file is boosted, because that is the one the turn is about.
    pub fn recall(&self, query: &str, touched: &[String]) -> Result<Vec<Memory>, Error> {
        let mut scored: Vec<(Memory, f32)> = Vec::new();
        let embedder = crate::embed::LocalEmbedder;
        let query_vec = embedder.embed(query);
        for memory in self.candidates(query)? {
            // A vector from another model lives in a different space, so it
            // scores as unrelated rather than as a false neighbour.
            let mut score = if self.embedder_of(memory.id)? == embedder.name() {
                crate::embed::cosine(&query_vec, &self.embedding(memory.id)?)
            } else {
                0.0
            };
            if self.linked_to(memory.id, touched)? {
                score += 0.5;
            }
            score += (memory.use_count as f32) * 0.01;
            scored.push((memory, score));
        }
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored
            .into_iter()
            .take(RECALL_LIMIT)
            .map(|(m, _)| m)
            .collect())
    }

    /// Every memory, newest use first. The browse view.
    pub fn list(&self) -> Result<Vec<Memory>, Error> {
        self.all()
    }

    /// A near-duplicate of `summary` already stored, if the embedding says so.
    pub fn similar(&self, summary: &str) -> Result<Option<Memory>, Error> {
        let query_vec = crate::embed::LocalEmbedder.embed(summary);
        let mut best: Option<(Memory, f32)> = None;
        for memory in self.all()? {
            let score = crate::embed::cosine(&query_vec, &self.embedding(memory.id)?);
            if score >= DEDUP_COSINE && best.as_ref().is_none_or(|(_, s)| score > *s) {
                best = Some((memory, score));
            }
        }
        Ok(best.map(|(m, _)| m))
    }

    fn candidates(&self, query: &str) -> Result<Vec<Memory>, Error> {
        let tokens: Vec<&str> = query.split_whitespace().filter(|t| t.len() >= 3).collect();
        if tokens.is_empty() {
            return self.all();
        }
        let match_query = tokens.join(" OR ");
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.category, m.summary, m.details, m.use_count
             FROM memories_fts f JOIN memories m ON m.id = f.rowid
             WHERE memories_fts MATCH ?1 LIMIT 50",
        )?;
        let rows = stmt.query_map(params![match_query], row_to_memory)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        if out.is_empty() { self.all() } else { Ok(out) }
    }

    fn all(&self) -> Result<Vec<Memory>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, category, summary, details, use_count FROM memories ORDER BY use_count DESC LIMIT 200",
        )?;
        let rows = stmt.query_map([], row_to_memory)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    fn id_for_hash(&self, hash: &str) -> Result<Option<i64>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM memories WHERE hash = ?1")?;
        let mut rows = stmt.query(params![hash])?;
        Ok(rows.next()?.map(|row| row.get(0)).transpose()?)
    }

    fn embedder_of(&self, id: i64) -> Result<String, Error> {
        self.conn
            .query_row(
                "SELECT embedder FROM memories WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(Error::from)
    }

    fn embedding(&self, id: i64) -> Result<Vec<f32>, Error> {
        let blob: Vec<u8> = self.conn.query_row(
            "SELECT embedding FROM memories WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(blob
            .chunks(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect())
    }

    fn link(&self, id: i64, paths: &[String]) -> Result<(), Error> {
        for path in paths {
            self.conn.execute(
                "INSERT OR IGNORE INTO memory_files (memory_id, path) VALUES (?1, ?2)",
                params![id, path],
            )?;
        }
        Ok(())
    }

    fn linked_to(&self, id: i64, touched: &[String]) -> Result<bool, Error> {
        if touched.is_empty() {
            return Ok(false);
        }
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM memory_files WHERE memory_id = ?1")?;
        let paths: Vec<String> = stmt
            .query_map(params![id], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(paths.iter().any(|p| touched.iter().any(|t| p == t)))
    }
}

fn row_to_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: row.get(0)?,
        category: row.get(1)?,
        summary: row.get(2)?,
        details: row.get(3)?,
        use_count: row.get(4)?,
    })
}

/// Adds a column the first release did not have. SQLite has no `ADD COLUMN
/// IF NOT EXISTS`, so the existing columns are read first.
fn ensure_column(conn: &Connection, name: &str, ddl: &str) -> Result<(), Error> {
    let mut stmt = conn.prepare("PRAGMA table_info(memories)")?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;
    if !names.iter().any(|n| n == name) {
        conn.execute_batch(&format!("ALTER TABLE memories ADD COLUMN {name} {ddl}"))?;
    }
    Ok(())
}

fn content_hash(summary: &str, details: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(summary.trim().to_lowercase());
    hasher.update([0]);
    hasher.update(details.trim().to_lowercase());
    format!("{:x}", hasher.finalize())
}

/// A hashed bag of character trigrams, L2-normalised.
///
/// Each trigram is hashed into a bucket and added with a sign, so unrelated
/// text cancels out and related text accumulates. No model, no vocabulary.
pub fn embed(text: &str) -> Vec<f32> {
    let mut vec = vec![0f32; EMBED_DIM];
    let lower: String = text
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let chars: Vec<char> = lower.chars().collect();
    if chars.len() < 3 {
        return vec;
    }
    for window in chars.windows(3) {
        let tri: String = window.iter().collect();
        let hash = hash_trigram(&tri);
        let bucket = (hash as usize) % EMBED_DIM;
        let sign = if hash & 1 == 0 { 1.0 } else { -1.0 };
        vec[bucket] += sign;
    }
    let norm = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }
    vec
}

fn hash_trigram(tri: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(tri.as_bytes());
    let bytes = hasher.finalize();
    u64::from_le_bytes(bytes[..8].try_into().unwrap_or([0; 8]))
}

fn bytemuck_vec(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Cosine similarity of two equal-length vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Renders a recall as the block injected into the system prompt.
pub fn render_recall(memories: &[Memory]) -> String {
    if memories.is_empty() {
        return String::new();
    }
    let mut out = String::from("# Recalled\n");
    for memory in memories {
        out.push_str(&format!("\n- [{}] {}", memory.category, memory.summary));
        if !memory.details.is_empty() {
            out.push_str(&format!(" — {}", memory.details));
        }
    }
    out
}

/// The arguments a `memory` tool call carries.
pub fn parse_remember(args: &Value) -> Option<(&str, &str, &str)> {
    let summary = args.get("summary")?.as_str()?;
    let details = args.get("details").and_then(Value::as_str).unwrap_or("");
    let category = args
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or("context");
    Some((category, summary, details))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx() -> (tempfile::TempDir, MemoryIndex) {
        let dir = tempfile::tempdir().unwrap();
        let index = MemoryIndex::open(dir.path()).unwrap();
        (dir, index)
    }

    #[test]
    fn the_same_text_is_stored_once() {
        let (_dir, index) = idx();
        let first = index.remember("decision", "use sqlite", "", &[]).unwrap();
        let second = index.remember("decision", "use sqlite", "", &[]).unwrap();
        assert!(matches!(first, Remembered::Added(_)));
        assert!(matches!(second, Remembered::Duplicate(_)));
        assert_eq!(index.all().unwrap().len(), 1);
        assert_eq!(index.all().unwrap()[0].use_count, 1);
    }

    #[test]
    fn recall_prefers_a_memory_about_the_file_being_edited() {
        let (_dir, index) = idx();
        index
            .remember(
                "gotcha",
                "auth expires early",
                "the token check uses <",
                &["src/auth.ts".into()],
            )
            .unwrap();
        index
            .remember("context", "the readme is long", "", &["README.md".into()])
            .unwrap();
        let recalled = index.recall("auth token", &["src/auth.ts".into()]).unwrap();
        assert_eq!(recalled[0].summary, "auth expires early");
    }

    #[test]
    fn similar_text_scores_higher_than_unrelated_text() {
        let auth = embed("the auth token expires too early");
        let auth_again = embed("auth token expiry is too early");
        let readme = embed("the readme explains the build");
        assert!(cosine(&auth, &auth_again) > cosine(&auth, &readme));
    }

    #[test]
    fn an_empty_index_recalls_nothing() {
        let (_dir, index) = idx();
        assert!(index.recall("anything", &[]).unwrap().is_empty());
    }
}
