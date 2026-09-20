//! UI-independent agent runtime shared by terminal, desktop, and headless surfaces.

pub mod agents;
pub mod protocol;
pub mod registry;
pub mod runtime;
pub mod tool_loop;

pub use agents::{AgentContext, AgentRequest, AgentRunner, AgentSupervisor, StreamingAgentRunner};
pub use protocol::{AgentKind, AgentStatus, EngineCommand, EngineEvent, TurnId};
pub use registry::{
    CredentialSource, EnvCredentialSource, HttpTransportFactory, LayeredCredentialSource,
    ModelDescriptor, ProviderDescriptor, ProviderRegistry, ProviderRegistryConfig, RegistryError,
    ResolvedModel, TransportFactory,
};
pub use runtime::{Engine, EngineConfig, EngineError, EngineRuntime, TransportResolver};
pub use tool_loop::TrajectorySink;

/// Crate version, mirrors the workspace release.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
