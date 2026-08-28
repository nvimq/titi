//! Terminal diff renderer: history batches, viewport diffing, overlays
//! (contract: `omp://tui-core-renderer`).

/// Crate version, mirrors the workspace release.

pub mod component;
pub mod caps;
pub mod cursor;
pub mod focus;
pub mod history;
pub mod image;
pub mod input;
pub mod keybindings;
pub mod keys;
pub mod overlay;
pub mod viewport;
pub mod width;
pub mod theme;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
