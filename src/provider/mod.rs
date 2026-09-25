//! Provider layer: service adapters and credential-aware construction.

pub mod localjev;
pub mod mock;
pub mod typesafe;

pub use duckjeu_judgment_core::provider::{
    validate_batch_result, CapabilityStatus, Provider, ProviderAnswer, ProviderBatchResult,
    ProviderCapabilities, ProviderMetadata, ProviderUsage,
};

use duckjeu_judgment_core::judgment::JudgmentError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Mock,
    Typesafe,
    SelfHosted,
}

impl ProviderKind {
    pub fn parse(raw: &str) -> Result<Self, JudgmentError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "mock" => Ok(Self::Mock),
            "typesafe" => Ok(Self::Typesafe),
            "selfhosted" => Ok(Self::SelfHosted),
            other => Err(JudgmentError::configuration(format!(
                "unknown duckjeu_provider '{other}': expected 'mock', 'typesafe', or 'selfhosted'"
            ))),
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Typesafe => "typesafe",
            Self::SelfHosted => "selfhosted",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderContext {
    pub provider: ProviderKind,
    pub api_url: String,
    pub model: String,
    pub timeout_ms: u64,
    pub max_response_bytes: u64,
    pub credential: Option<String>,
}

pub const DEFAULT_API_URL: &str = "https://api.typesafe.ai/v1/systemone";
pub const DEFAULT_MODEL: &str = "jev-latest";
pub const DEFAULT_LOCAL_JEV_API_URL: &str = "http://127.0.0.1:8765/v1/systemone";
pub const DEFAULT_LOCAL_JEV_MODEL: &str = "nli-deberta-large";
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_MAX_RESPONSE_BYTES: u64 = 1_048_576;

impl Default for ProviderContext {
    fn default() -> Self {
        Self {
            provider: ProviderKind::Mock,
            api_url: DEFAULT_API_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            credential: None,
        }
    }
}

pub fn provider_for(ctx: &ProviderContext) -> Result<Box<dyn Provider>, JudgmentError> {
    match ctx.provider {
        ProviderKind::Mock => Ok(Box::new(mock::MockProvider)),
        ProviderKind::Typesafe => {
            let credential = ctx
                .credential
                .as_deref()
                .filter(|value| !value.trim().is_empty());
            match credential {
                Some(credential) => Ok(Box::new(typesafe::TypesafeProvider::new(
                    ctx.api_url.clone(),
                    ctx.model.clone(),
                    ctx.timeout_ms,
                    ctx.max_response_bytes,
                    credential.to_string(),
                ))),
                None => Err(JudgmentError::configuration(
                    "duckjeu_provider='typesafe' requires the TYPESAFE_API_KEY environment variable",
                )),
            }
        }
        ProviderKind::SelfHosted => Ok(Box::new(localjev::LocalJevProvider::new(
            ctx.api_url.clone(),
            ctx.model.clone(),
            ctx.timeout_ms,
            ctx.max_response_bytes,
            ctx.credential.clone(),
        ))),
    }
}
