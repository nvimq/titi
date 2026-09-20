//! Auth store: the single issuer of API keys and OAuth tokens.
//!
//! SQLite database at `<agent_dir>/auth.db`; the file is restricted to
//! `0600` (owner read/write only) so credentials never leak to other users.
//!
//! Spec: docs/research/secrets-env/README.md (omp AuthStorage analogue).

use std::fmt;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// Store error: filesystem or database failure, both opaque to callers.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Db(rusqlite::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "auth store io error: {e}"),
            Error::Db(e) => write!(f, "auth store db error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Db(e) => Some(e),
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Db(e)
    }
}

/// One stored credential for a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub provider: String,
    /// e.g. `oauth` or `api_key`.
    pub kind: String,
    pub token: String,
    /// Unix seconds; `None` = never expires.
    pub expires_at: Option<i64>,
    pub updated_at: i64,
}

/// SQLite-backed credential store (`credentials` table, upsert semantics).
pub struct AuthStore {
    conn: rusqlite::Connection,
}

impl AuthStore {
    /// Open (creating if needed) the database at `path` with `0600` permissions.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Create the file ourselves with mode 0600 so SQLite never races a
        // world-readable default.
        let mut opts = fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        opts.mode(0o600);
        opts.open(path)?;

        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS credentials (
                provider   TEXT PRIMARY KEY,
                kind       TEXT NOT NULL,
                token      TEXT NOT NULL,
                expires_at INTEGER,
                updated_at INTEGER NOT NULL
            );",
        )?;
        #[cfg(unix)]
        restrict_permissions(path)?;
        Ok(Self { conn })
    }

    /// Insert or update a credential for its provider.
    pub fn store(
        &self,
        provider: &str,
        kind: &str,
        token: &str,
        expires_at: Option<i64>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO credentials (provider, kind, token, expires_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(provider) DO UPDATE SET
                kind = excluded.kind,
                token = excluded.token,
                expires_at = excluded.expires_at,
                updated_at = excluded.updated_at",
            rusqlite::params![provider, kind, token, expires_at, now()],
        )?;
        Ok(())
    }

    /// Fetch the credential for `provider`, if any.
    pub fn get(&self, provider: &str) -> Result<Option<Credential>> {
        let mut stmt = self.conn.prepare(
            "SELECT provider, kind, token, expires_at, updated_at
             FROM credentials WHERE provider = ?1",
        )?;
        let mut rows = stmt.query([provider])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_credential(row)?)),
            None => Ok(None),
        }
    }

    /// Remove the credential for `provider`; returns whether a row existed.
    pub fn remove(&self, provider: &str) -> Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM credentials WHERE provider = ?1", [provider])?;
        Ok(n > 0)
    }

    /// All stored credentials, ordered by provider.
    pub fn list(&self) -> Result<Vec<Credential>> {
        let mut stmt = self.conn.prepare(
            "SELECT provider, kind, token, expires_at, updated_at
             FROM credentials ORDER BY provider",
        )?;
        let rows = stmt.query_map([], row_to_credential)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Quarantine dead credentials: drop every row whose `expires_at` has
    /// passed (e.g. refresh tokens that can no longer be renewed).
    /// Returns the number of quarantined rows.
    pub fn quarantine_expired(&self) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM credentials WHERE expires_at IS NOT NULL AND expires_at < ?1",
            [now()],
        )?;
        Ok(n)
    }
}

fn row_to_credential(row: &rusqlite::Row<'_>) -> rusqlite::Result<Credential> {
    Ok(Credential {
        provider: row.get(0)?,
        kind: row.get(1)?,
        token: row.get(2)?,
        expires_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_tmp(tag: &str) -> (tempfile::TempDir, AuthStore) {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir failed: {e}"));
        let store =
            AuthStore::open(&dir.path().join(tag)).unwrap_or_else(|e| panic!("open failed: {e}"));
        (dir, store)
    }

    #[test]
    fn store_get_remove_roundtrip() {
        let (_dir, store) = open_tmp("auth.db");
        assert_eq!(
            store
                .get("anthropic")
                .unwrap_or_else(|e| panic!("get: {e}")),
            None
        );

        store
            .store("anthropic", "oauth", "tok-1", None)
            .unwrap_or_else(|e| panic!("store: {e}"));
        let cred = store
            .get("anthropic")
            .unwrap_or_else(|e| panic!("get: {e}"));
        assert_eq!(cred.as_ref().map(|c| c.token.as_str()), Some("tok-1"));
        assert_eq!(cred.as_ref().map(|c| c.kind.as_str()), Some("oauth"));
        assert_eq!(cred.as_ref().and_then(|c| c.expires_at), None);
        assert!(cred.is_some_and(|c| c.updated_at > 0));

        // Upsert replaces token and expiry.
        store
            .store("anthropic", "api_key", "tok-2", Some(1_700_000_000))
            .unwrap_or_else(|e| panic!("store: {e}"));
        let cred = store
            .get("anthropic")
            .unwrap_or_else(|e| panic!("get: {e}"));
        assert_eq!(cred.as_ref().map(|c| c.token.as_str()), Some("tok-2"));
        assert_eq!(
            cred.as_ref().and_then(|c| c.expires_at),
            Some(1_700_000_000)
        );

        assert!(store.list().unwrap_or_else(|e| panic!("list: {e}")).len() == 1);
        assert!(
            store
                .remove("anthropic")
                .unwrap_or_else(|e| panic!("remove: {e}"))
        );
        assert!(
            !store
                .remove("anthropic")
                .unwrap_or_else(|e| panic!("remove: {e}"))
        );
        assert_eq!(
            store
                .get("anthropic")
                .unwrap_or_else(|e| panic!("get: {e}")),
            None
        );
    }

    #[test]
    fn list_and_remove_are_provider_scoped() {
        let (_dir, store) = open_tmp("auth.db");
        store
            .store("a", "api_key", "ta", None)
            .unwrap_or_else(|e| panic!("store: {e}"));
        store
            .store("b", "oauth", "tb", None)
            .unwrap_or_else(|e| panic!("store: {e}"));
        assert!(store.remove("a").unwrap_or_else(|e| panic!("remove: {e}")));
        let listed = store.list().unwrap_or_else(|e| panic!("list: {e}"));
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].provider, "b");
    }

    #[test]
    fn db_file_is_owner_only_0600() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir failed: {e}"));
        let path = dir.path().join("auth.db");
        AuthStore::open(&path).unwrap_or_else(|e| panic!("open failed: {e}"));
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path)
            .unwrap_or_else(|e| panic!("stat: {e}"))
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "db file must be 0600, got {mode:o}");
    }

    #[test]
    fn reopening_existing_db_preserves_rows_and_fixes_mode() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir failed: {e}"));
        let path = dir.path().join("auth.db");
        {
            let store = AuthStore::open(&path).unwrap_or_else(|e| panic!("open: {e}"));
            store
                .store("openai", "api_key", "k", None)
                .unwrap_or_else(|e| panic!("store: {e}"));
        }
        // Simulate a wrong-mode pre-existing file, then reopen: mode is repaired.
        let mut perms = fs::metadata(&path)
            .unwrap_or_else(|e| panic!("stat: {e}"))
            .permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&path, perms).unwrap_or_else(|e| panic!("chmod: {e}"));
        let store = AuthStore::open(&path).unwrap_or_else(|e| panic!("reopen: {e}"));
        assert_eq!(
            store
                .get("openai")
                .unwrap_or_else(|e| panic!("get: {e}"))
                .map(|c| c.token),
            Some("k".to_string())
        );
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path)
            .unwrap_or_else(|e| panic!("stat: {e}"))
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn quarantine_expired_drops_only_dead_rows() {
        let (_dir, store) = open_tmp("auth.db");
        let now = now();
        store
            .store("dead-refresh", "oauth", "r1", Some(now - 100))
            .unwrap_or_else(|e| panic!("store: {e}"));
        store
            .store("live-refresh", "oauth", "r2", Some(now + 3_600))
            .unwrap_or_else(|e| panic!("store: {e}"));
        store
            .store("never-expires", "api_key", "k", None)
            .unwrap_or_else(|e| panic!("store: {e}"));

        assert_eq!(
            store
                .quarantine_expired()
                .unwrap_or_else(|e| panic!("quarantine: {e}")),
            1
        );
        assert!(
            !store
                .remove("dead-refresh")
                .unwrap_or_else(|e| panic!("remove: {e}"))
        );
        assert!(
            store
                .get("live-refresh")
                .unwrap_or_else(|e| panic!("get: {e}"))
                .is_some()
        );
        assert!(
            store
                .get("never-expires")
                .unwrap_or_else(|e| panic!("get: {e}"))
                .is_some()
        );
        // Idempotent: nothing left to quarantine.
        assert_eq!(
            store
                .quarantine_expired()
                .unwrap_or_else(|e| panic!("quarantine: {e}")),
            0
        );
    }
}
