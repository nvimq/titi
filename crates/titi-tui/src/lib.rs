//! Terminal diff renderer: history batches, viewport diffing, overlays
//! (contract: `omp://tui-core-renderer`).

/// Crate version, mirrors the workspace release.

pub mod component;
pub mod caps;
pub mod composer;
pub mod cursor;
pub mod focus;
pub mod history;
pub mod hub;
pub mod image;
pub mod input;
pub mod keybindings;
pub mod keys;
pub mod markdown;
pub mod overlay;
pub mod panels;
pub mod renderer;
pub mod viewport;
pub mod space_hold;
pub mod slash;
pub mod status;
pub mod status_bar;
pub mod width;
pub mod theme;
pub mod transcript;
pub mod selection;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
