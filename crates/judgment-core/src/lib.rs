//! Host-independent judgment identities, serialization, validation, caching, and execution.
//!
//! This crate deliberately has no DuckDB dependency and stores no credentials or host objects.

pub mod cache;
pub mod executor;
pub mod judgment;
pub mod metrics;
pub mod provider;
pub mod runtime;
pub mod serialize;

pub use cache::{CacheCounters, CachePolicy, JudgmentCache};
pub use judgment::{
    bool_from_probability, validate_judgment_result, validate_label, validate_probability,
    JudgmentError, JudgmentIdentity, JudgmentIdentityContext, JudgmentRequest, JudgmentResult,
    QuestionKind,
};
pub use provider::{
    validate_batch_result, CapabilityStatus, Provider, ProviderAnswer, ProviderBatchResult,
    ProviderCapabilities, ProviderMetadata, ProviderUsage,
};
pub use serialize::{CanonicalState, StateKind, TypedValue};
