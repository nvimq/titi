//! Credential administration for the CLI: `--set-key` / `--list-keys`.
//!
//! Keys land in `<agent_dir>/auth.db`, the store `LayeredCredentialSource`
//! already consults after the environment and `.env` layers. Tokens are never
//! printed back — listing shows provider ids and kinds only.

use std::path::Path;

use titi_secrets::store::AuthStore;

/// One stored credential, without its token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredKey {
    pub provider: String,
    pub kind: String,
    pub updated_at: i64,
}

/// Stores an API key for `provider`.
pub fn store_key(agent_dir: &Path, provider: &str, key: &str) -> Result<(), String> {
    let provider = provider.trim();
    let key = key.trim();
    if provider.is_empty() {
        return Err("a provider id is required".into());
    }
    if key.is_empty() {
        return Err("a key value is required".into());
    }
    let store = AuthStore::open(&agent_dir.join("auth.db")).map_err(|e| e.to_string())?;
    store
        .store(provider, "api_key", key, None)
        .map_err(|e| e.to_string())
}

/// Every stored credential's provider id, kind and timestamp. No tokens.
pub fn list_keys(agent_dir: &Path) -> Result<Vec<StoredKey>, String> {
    let store = AuthStore::open(&agent_dir.join("auth.db")).map_err(|e| e.to_string())?;
    store.list().map_err(|e| e.to_string()).map(|rows| {
        rows.into_iter()
            .map(|row| StoredKey {
                provider: row.provider,
                kind: row.kind,
                updated_at: row.updated_at,
            })
            .collect()
    })
}
