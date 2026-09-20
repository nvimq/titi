//! Schema-agnostic config file loader: `.yml`/`.yaml`/`.json`/`.jsonc`,
//! tri-state outcome, JSON→YAML migration.

use serde::de::DeserializeOwned;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid config in {path}: {reason}")]
    Invalid { path: PathBuf, reason: String },
    #[error("io error for {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Tri-state load result, mirroring `ConfigFile.tryLoad()` in omp.
pub enum LoadOutcome<T> {
    Ok(T),
    NotFound,
    Error(ConfigError),
}

/// Load and deserialize a single config file.
///
/// - `.json` / `.jsonc`: parsed as JSON; `.jsonc` first strips comments.
/// - `.yml` / `.yaml`: if the target is missing but a sibling `.json` exists,
///   it is migrated once (YAML written, JSON renamed to `.json.bak`).
pub fn try_load<T: DeserializeOwned>(path: &Path) -> LoadOutcome<T> {
    match fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if migrate_json_to_yaml::<T>(path) {
                match fs::read_to_string(path) {
                    Ok(text) => parse_yaml(path, &text),
                    Err(e) => LoadOutcome::Error(ConfigError::Io {
                        path: path.into(),
                        source: e,
                    }),
                }
            } else {
                LoadOutcome::NotFound
            }
        }
        Err(e) => LoadOutcome::Error(ConfigError::Io {
            path: path.into(),
            source: e,
        }),
        Ok(text) => parse_yaml(path, &text),
    }
}

fn parse_yaml<T: DeserializeOwned>(path: &Path, text: &str) -> LoadOutcome<T> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext == "json" || ext == "jsonc" {
        let stripped = if ext == "jsonc" {
            strip_jsonc_comments(text)
        } else {
            text.to_owned()
        };
        match serde_json::from_str::<T>(&stripped) {
            Ok(v) => LoadOutcome::Ok(v),
            Err(e) => LoadOutcome::Error(ConfigError::Invalid {
                path: path.into(),
                reason: e.to_string(),
            }),
        }
    } else {
        match serde_yaml::from_str::<T>(text) {
            Ok(v) => LoadOutcome::Ok(v),
            Err(e) => LoadOutcome::Error(ConfigError::Invalid {
                path: path.into(),
                reason: e.to_string(),
            }),
        }
    }
}

/// One-shot JSON→YAML migration for YAML-targeted paths.
fn migrate_json_to_yaml<T: DeserializeOwned>(yaml_path: &Path) -> bool {
    let ext = yaml_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext != "yml" && ext != "yaml" {
        return false;
    }
    let json_path = yaml_path.with_extension("json");
    let Ok(text) = fs::read_to_string(&json_path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&strip_jsonc_comments(&text)) else {
        return false;
    };
    let Ok(yaml) = serde_yaml::to_string(&value) else {
        return false;
    };
    if fs::write(yaml_path, yaml).is_err() {
        return false;
    }
    let _ = fs::rename(&json_path, json_path.with_extension("json.bak"));
    true
}

/// Strip `//` line comments and `/* */` block comments, keeping string literals intact.
pub(crate) fn strip_jsonc_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut closed = false;
                while let Some(n) = chars.next() {
                    if n == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    break;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Acquire an exclusive advisory lock on `<path>.lock` while holding the guard.
pub fn with_file_lock<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    let lock_path = path.with_extension({
        let mut s = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_owned();
        s.push_str(".lock");
        s
    });
    if let Some(parent) = lock_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let opened = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path);
    let result = match opened {
        Ok(file) => {
            let mut guard = fd_lock::RwLock::new(file);
            match guard.write() {
                Ok(_w) => f(),
                Err(_) => f(),
            }
        }
        Err(_) => f(),
    };
    let _ = fs::remove_file(&lock_path);
    result
}
