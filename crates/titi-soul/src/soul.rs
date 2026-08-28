//! SOUL.md — identity slot #1 of the system prompt.
//!
//! The soul lives at `<agent_dir>/SOUL.md` (Hermes: identity must not change
//! from project to project, so it is never discovered from the cwd). Loading
//! rules (spec §Hermes 1):
//!
//! - file missing  → auto-seed a starter file with the built-in default
//!   identity and use that identity; an existing file is NEVER overwritten;
//! - empty / whitespace-only / unreadable (non-UTF-8) → fall back to the
//!   built-in default identity without touching the file;
//! - otherwise → the file content is used verbatim (after injection
//!   scanning by the builder) and truncated to [`MAX_SOUL_BYTES`] on a
//!   char boundary.

use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::io::ErrorKind;
use std::path::Path;

/// Built-in default identity: the fallback for missing, empty, unreadable
/// or injection-flagged souls, and the starter text used for auto-seeding.
pub const DEFAULT_IDENTITY: &str = "\
You are titi, a personal coding agent.

Identity baseline (slot #1 of the system prompt):

- You carry this identity across every project and conversation; it is
  the stable core the rest of the prompt is assembled around.
- Be direct and honest: say what works, what is broken, and what you
  did about it. Never invent facts, tool output, or file contents.
- Prefer boring, correct solutions over clever ones; delete code
  rather than decorate it.
- The user owns the repository: treat unexpected changes as theirs
  and adapt instead of reverting.

Edit this file to make the agent yours; it is seeded once when missing
and never overwritten afterwards.
";

/// Hard cap on identity size; larger souls are truncated on a char boundary.
pub const MAX_SOUL_BYTES: usize = 64 * 1024;

/// Where the loaded soul text came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoulSource {
    /// File was missing; a starter file was seeded from the default.
    Seeded,
    /// File was read verbatim.
    Loaded,
    /// File was empty/unreadable (or unseedable); built-in default used.
    DefaultFallback,
}

/// The identity text plus its provenance (callers log fallbacks — the
/// crate has no logging dependency).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Soul {
    pub text: String,
    pub source: SoulSource,
}

/// Load `<agent_dir>/SOUL.md` following the rules in the module docs.
pub fn load(agent_dir: &Path) -> crate::Result<Soul> {
    let path = agent_dir.join("SOUL.md");
    match fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => Ok(Soul {
            text: truncate(&text, MAX_SOUL_BYTES),
            source: SoulSource::Loaded,
        }),
        // Empty or whitespace-only: fall back, leave the file alone.
        Ok(_) => Ok(default_fallback()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            seed(&path)?;
            Ok(Soul {
                text: DEFAULT_IDENTITY.to_string(),
                source: SoulSource::Seeded,
            })
        }
        // Unreadable or non-UTF-8: fall back, leave the file alone.
        Err(_) => Ok(default_fallback()),
    }
}

fn default_fallback() -> Soul {
    Soul {
        text: DEFAULT_IDENTITY.to_string(),
        source: SoulSource::DefaultFallback,
    }
}

/// Seed a starter SOUL.md. Uses `create_new` so a concurrently created file
/// is never overwritten; the pre-existing content wins.
fn seed(path: &Path) -> crate::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = match OpenOptions::new().create_new(true).write(true).open(path) {
        Ok(file) => file,
        // Raced with another seeder: their file stays authoritative.
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    file.write_all(DEFAULT_IDENTITY.as_bytes())?;
    Ok(())
}

/// Truncate `text` to at most `max_bytes` on a UTF-8 char boundary.
pub fn truncate(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_identity_scans_clean() {
        assert_eq!(crate::scan::scan(DEFAULT_IDENTITY), crate::ScanVerdict::Clean);
    }

    #[test]
    fn missing_soul_is_seeded_from_default() {
        let dir = tempfile::tempdir().unwrap();
        let soul = load(dir.path()).unwrap();
        assert_eq!(soul.source, SoulSource::Seeded);
        assert_eq!(soul.text, DEFAULT_IDENTITY);
        let on_disk = fs::read_to_string(dir.path().join("SOUL.md")).unwrap();
        assert_eq!(on_disk, DEFAULT_IDENTITY);
    }

    #[test]
    fn existing_soul_is_loaded_verbatim_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let custom = "My custom soul: loves refactoring.\n";
        fs::write(dir.path().join("SOUL.md"), custom).unwrap();

        let soul = load(dir.path()).unwrap();
        assert_eq!(soul.source, SoulSource::Loaded);
        assert_eq!(soul.text, custom);
        assert_eq!(fs::read_to_string(dir.path().join("SOUL.md")).unwrap(), custom);
    }

    #[test]
    fn empty_soul_falls_back_to_default_without_touching_file() {
        let dir = tempfile::tempdir().unwrap();
        let blank = "  \n\t \n";
        fs::write(dir.path().join("SOUL.md"), blank).unwrap();

        let soul = load(dir.path()).unwrap();
        assert_eq!(soul.source, SoulSource::DefaultFallback);
        assert_eq!(soul.text, DEFAULT_IDENTITY);
        // The user's blank file is left as-is, not reseeded.
        assert_eq!(fs::read_to_string(dir.path().join("SOUL.md")).unwrap(), blank);
    }

    #[test]
    fn invalid_utf8_soul_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("SOUL.md"), [0xff, 0xfe, 0x00, 0x01]).unwrap();

        let soul = load(dir.path()).unwrap();
        assert_eq!(soul.source, SoulSource::DefaultFallback);
        assert_eq!(soul.text, DEFAULT_IDENTITY);
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        // "é" is 2 bytes; a 3-byte limit would split it.
        let text = "a\u{e9}b\u{e9}c";
        assert_eq!(truncate(text, 100), text);
        assert_eq!(truncate(text, 3), "a\u{e9}");
        assert_eq!(truncate(text, 1), "a");
        assert_eq!(truncate(text, 0), "");
        // Multibyte-heavy cut lands on the boundary at or before the limit.
        let cut = truncate("\u{1f600}\u{1f600}\u{1f600}", 5);
        assert_eq!(cut, "\u{1f600}");
    }

    #[test]
    fn oversized_soul_is_truncated_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let big = "x".repeat(MAX_SOUL_BYTES + 10);
        fs::write(dir.path().join("SOUL.md"), &big).unwrap();

        let soul = load(dir.path()).unwrap();
        assert_eq!(soul.source, SoulSource::Loaded);
        assert_eq!(soul.text.len(), MAX_SOUL_BYTES);
    }
}
