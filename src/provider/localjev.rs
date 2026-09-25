//! Adapter for the pinned local-jev System One protocol.

use duckjeu_judgment_core::judgment::{JudgmentError, JudgmentRequest, JudgmentResult};

use super::typesafe::TypesafeProvider;
use super::{Provider, ProviderBatchResult, ProviderCapabilities};

pub struct LocalJevProvider(TypesafeProvider);

impl LocalJevProvider {
    pub fn new(
        api_url: String,
        model: String,
        timeout_ms: u64,
        max_response_bytes: u64,
        credential: Option<String>,
    ) -> Self {
        Self(TypesafeProvider::new_localjev(
            api_url,
            model,
            timeout_ms,
            max_response_bytes,
            credential,
        ))
    }
}

impl Provider for LocalJevProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        self.0.capabilities()
    }
    fn makes_external_request(&self) -> bool {
        true
    }
    fn judge(&self, request: &JudgmentRequest) -> Result<JudgmentResult, JudgmentError> {
        self.0.judge(request)
    }
    fn judge_with_metadata(
        &self,
        request: &JudgmentRequest,
    ) -> Result<ProviderBatchResult, JudgmentError> {
        self.0.judge_with_metadata(request)
    }
    fn cache_model_identity(&self, requested_model: &str) -> Option<String> {
        self.0.cache_model_identity(requested_model)
    }
    fn estimate_request_bytes(&self, requests: &[JudgmentRequest]) -> usize {
        self.0.estimate_request_bytes(requests)
    }
    fn judge_many(
        &self,
        requests: &[JudgmentRequest],
    ) -> Result<ProviderBatchResult, JudgmentError> {
        self.0.judge_many(requests)
    }
}
