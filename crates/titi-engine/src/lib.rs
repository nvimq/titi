//! UI-independent agent runtime shared by terminal, desktop, and headless surfaces.

pub mod agents;
pub mod protocol;
pub mod runtime;

pub use agents::{AgentContext, AgentRequest, AgentRunner, AgentSupervisor};
pub use protocol::{AgentKind, AgentStatus, EngineCommand, EngineEvent, TurnId};
pub use runtime::{Engine, EngineConfig, EngineError, EngineRuntime, TransportResolver};

/// Crate version, mirrors the workspace release.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
