//! Secrets and credentials: layered `.env` resolution, auth store.
//! Spec: docs/research/secrets-env/README.md (omp secrets + Hermes /reload + Vellum creds-process).

/// Crate version, mirrors the workspace release.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod env;
pub mod store;
