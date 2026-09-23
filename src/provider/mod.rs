//! Provider 层：只承担服务协议、认证接入、能力声明与响应转换。
//!
//! 不负责写回 DuckDB 向量（见根规格 §9.1）。

pub mod mock;
pub mod typesafe;

use crate::judgment::{JudgmentError, JudgmentRequest, JudgmentResult};

/// 支持的 provider 种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    /// 确定性离线 mock（默认）。
    Mock,
    /// TypeSafe System One 真实服务（需显式开启）。
    Typesafe,
}

impl ProviderKind {
    pub fn parse(raw: &str) -> Result<Self, JudgmentError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "mock" => Ok(ProviderKind::Mock),
            "typesafe" => Ok(ProviderKind::Typesafe),
            other => Err(JudgmentError::configuration(format!(
                "unknown duckjeu_provider '{other}': expected 'mock' or 'typesafe'"
            ))),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Mock => "mock",
            ProviderKind::Typesafe => "typesafe",
        }
    }
}

/// 连接级配置在一次执行中冻结后的快照。
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderContext {
    pub provider: ProviderKind,
    pub api_url: String,
    pub model: String,
    pub timeout_ms: u64,
    pub max_response_bytes: u64,
    /// 凭据只存在于内存上下文与 Authorization header。
    pub credential: Option<String>,
}

pub const DEFAULT_API_URL: &str = "https://api.typesafe.ai/v1/systemone";
pub const DEFAULT_MODEL: &str = "jev-latest";
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_MAX_RESPONSE_BYTES: u64 = 1_048_576;

impl Default for ProviderContext {
    fn default() -> Self {
        ProviderContext {
            provider: ProviderKind::Mock,
            api_url: DEFAULT_API_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            credential: None,
        }
    }
}

/// provider 契约：mock 与真实 adapter 经过同一验证路径。
pub trait Provider: Send + Sync {
    fn judge(&self, request: &JudgmentRequest) -> Result<JudgmentResult, JudgmentError>;
}

/// 按上下文选择 adapter。真实 provider 需要显式开启且必须有凭据。
pub fn provider_for(ctx: &ProviderContext) -> Result<Box<dyn Provider>, JudgmentError> {
    match ctx.provider {
        ProviderKind::Mock => Ok(Box::new(mock::MockProvider)),
        ProviderKind::Typesafe => {
            let credential = ctx.credential.as_deref().filter(|c| !c.trim().is_empty());
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
    }
}
