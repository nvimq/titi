//! Bounded memory stores (MEMORY/USER) with capacity management and security scan.
//! Spec: docs/research/memory-learning/README.md (+ stores.md)

/// Crate version, mirrors the workspace release.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod embed;
pub mod index;
pub mod sanitize;
pub mod store;
pub mod tool;
