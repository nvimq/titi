use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use titi_providers::{
    ApiKind, CredKind, Credential, FamilyTransport, LadderLevel, Transport, TransportError,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub id: SmolStr,
    pub api: ApiKind,
    pub base_url: SmolStr,
    pub credential_env: Option<SmolStr>,
    #[serde(default = "default_true")]
    pub credential_required: bool,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    pub id: SmolStr,
    pub provider: SmolStr,
    pub wire_model: SmolStr,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRegistryConfig {
    pub providers: Vec<ProviderDescriptor>,
    pub models: Vec<ModelDescriptor>,
}

impl ProviderRegistryConfig {
    pub fn from_settings_value(value: &serde_json::Value) -> Option<Self> {
        let parsed: Self = serde_json::from_value(value.clone()).ok()?;
        if parsed.providers.is_empty() || parsed.models.is_empty() {
            return None;
        }
        Some(parsed)
    }
}

pub struct ResolvedModel {
    pub id: SmolStr,
    pub wire_model: SmolStr,
    pub transport: Arc<dyn Transport>,
    pub credential: Option<Credential>,
}

impl ResolvedModel {
    pub fn without_credential(id: impl Into<SmolStr>, transport: Arc<dyn Transport>) -> Self {
        let id = id.into();
        Self {
            id: id.clone(),
            wire_model: id,
            transport,
            credential: None,
        }
    }
}

pub trait CredentialSource: Send + Sync + 'static {
    fn resolve(&self, provider: &ProviderDescriptor) -> Option<Credential>;
}

#[derive(Debug, Default)]
pub struct EnvCredentialSource;

impl CredentialSource for EnvCredentialSource {
    fn resolve(&self, provider: &ProviderDescriptor) -> Option<Credential> {
        let key = provider.credential_env.as_deref()?;
        let access = std::env::var(key).ok()?;
        if access.trim().is_empty() {
            return None;
        }
        Some(Credential {
            access: access.into(),
            kind: CredKind::ApiKey,
            level: LadderLevel::Env,
        })
    }
}

/// Env, layered `.env`, then `auth.db` for the provider id.
pub struct LayeredCredentialSource {
    env: titi_secrets::env::LayeredEnv,
    store_path: std::path::PathBuf,
}

impl LayeredCredentialSource {
    pub fn from_defaults() -> Self {
        let agent_dir = titi_config::agent_dir();
        Self {
            env: titi_secrets::env::LayeredEnv::from_defaults(),
            store_path: agent_dir.join("auth.db"),
        }
    }
}

impl CredentialSource for LayeredCredentialSource {
    fn resolve(&self, provider: &ProviderDescriptor) -> Option<Credential> {
        if let Some(key) = provider.credential_env.as_deref()
            && let Some(access) = self.env.resolve(key)
            && !access.trim().is_empty()
        {
            return Some(Credential {
                access: access.into(),
                kind: CredKind::ApiKey,
                level: LadderLevel::Env,
            });
        }
        let store = titi_secrets::store::AuthStore::open(&self.store_path).ok()?;
        let stored = store.get(provider.id.as_str()).ok()??;
        if stored.token.trim().is_empty() {
            return None;
        }
        Some(Credential {
            access: stored.token.into(),
            kind: if stored.kind == "oauth" {
                CredKind::BearerToken
            } else {
                CredKind::ApiKey
            },
            level: LadderLevel::Stored,
        })
    }
}

pub trait TransportFactory: Send + Sync + 'static {
    fn build(&self, provider: &ProviderDescriptor) -> Result<Arc<dyn Transport>, TransportError>;
}

#[derive(Debug, Default)]
pub struct HttpTransportFactory;

impl TransportFactory for HttpTransportFactory {
    fn build(&self, provider: &ProviderDescriptor) -> Result<Arc<dyn Transport>, TransportError> {
        FamilyTransport::with_default_fetch(provider.api, provider.base_url.clone())
            .map(|transport| Arc::new(transport) as Arc<dyn Transport>)
    }
}

struct ProviderEntry {
    descriptor: ProviderDescriptor,
    transport: Arc<dyn Transport>,
}

pub struct ProviderRegistry {
    providers: HashMap<SmolStr, ProviderEntry>,
    models: HashMap<SmolStr, ModelDescriptor>,
    credentials: Arc<dyn CredentialSource>,
}

impl ProviderRegistry {
    pub fn new(
        config: ProviderRegistryConfig,
        credentials: Arc<dyn CredentialSource>,
        transports: Arc<dyn TransportFactory>,
    ) -> Result<Self, RegistryError> {
        let mut providers = HashMap::new();
        for descriptor in config.providers {
            if providers.contains_key(&descriptor.id) {
                return Err(RegistryError::DuplicateProvider(descriptor.id));
            }
            let transport =
                transports
                    .build(&descriptor)
                    .map_err(|source| RegistryError::Transport {
                        provider: descriptor.id.clone(),
                        source,
                    })?;
            providers.insert(
                descriptor.id.clone(),
                ProviderEntry {
                    descriptor,
                    transport,
                },
            );
        }

        let mut models = HashMap::new();
        for model in config.models {
            if !providers.contains_key(&model.provider) {
                return Err(RegistryError::UnknownProvider {
                    model: model.id,
                    provider: model.provider,
                });
            }
            if models.insert(model.id.clone(), model.clone()).is_some() {
                return Err(RegistryError::DuplicateModel(model.id));
            }
        }

        Ok(Self {
            providers,
            models,
            credentials,
        })
    }

    pub fn model_ids(&self) -> Vec<SmolStr> {
        let mut ids: Vec<_> = self.models.keys().cloned().collect();
        ids.sort();
        ids
    }

    pub fn resolve(&self, model_id: &str) -> Result<ResolvedModel, RegistryError> {
        let model = self
            .models
            .get(model_id)
            .ok_or_else(|| RegistryError::UnknownModel(model_id.into()))?;
        let provider =
            self.providers
                .get(&model.provider)
                .ok_or_else(|| RegistryError::UnknownProvider {
                    model: model.id.clone(),
                    provider: model.provider.clone(),
                })?;
        let credential = self.credentials.resolve(&provider.descriptor);
        if provider.descriptor.credential_required && credential.is_none() {
            return Err(RegistryError::MissingCredential {
                provider: provider.descriptor.id.clone(),
                env: provider.descriptor.credential_env.clone(),
            });
        }
        Ok(ResolvedModel {
            id: model.id.clone(),
            wire_model: model.wire_model.clone(),
            transport: Arc::clone(&provider.transport),
            credential,
        })
    }
}

impl crate::runtime::TransportResolver for ProviderRegistry {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, RegistryError> {
        ProviderRegistry::resolve(self, model)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("unknown model {0}")]
    UnknownModel(SmolStr),
    #[error("model {model} references unknown provider {provider}")]
    UnknownProvider { model: SmolStr, provider: SmolStr },
    #[error("duplicate provider {0}")]
    DuplicateProvider(SmolStr),
    #[error("duplicate model {0}")]
    DuplicateModel(SmolStr),
    #[error("provider {provider} requires a credential from {env:?}")]
    MissingCredential {
        provider: SmolStr,
        env: Option<SmolStr>,
    },
    #[error("failed to build transport for {provider}: {source}")]
    Transport {
        provider: SmolStr,
        #[source]
        source: TransportError,
    },
}
