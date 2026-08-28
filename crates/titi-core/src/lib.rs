//! Agent runtime: sessions, agent loop, memory, trajectory capture.

pub mod compaction;
pub mod context_files;
pub mod prewalk;
pub mod session;
pub mod trajectory;

/// Crate version, mirrors the workspace release.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
