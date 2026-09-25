//! Host-independent provider protocol and capability declarations.

use std::collections::HashSet;

use crate::judgment::{
    validate_label, validate_probability, JudgmentError, JudgmentRequest, JudgmentResult,
    QuestionKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityStatus {
    Verified,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub protocol_version: String,
    pub noul: CapabilityStatus,
    pub choice: CapabilityStatus,
    pub multi_state_batch: CapabilityStatus,
    pub max_batch_judgments: Option<usize>,
    pub max_choices: Option<usize>,
}

impl ProviderCapabilities {
    pub fn supports(&self, kind: QuestionKind) -> CapabilityStatus {
        match kind {
            QuestionKind::Noul => self.noul,
            QuestionKind::Choice => self.choice,
        }
    }
    pub fn supports_multi_state_batch(&self) -> bool {
        self.multi_state_batch == CapabilityStatus::Verified
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderMetadata {
    pub requested_model: Option<String>,
    pub effective_model: Option<String>,
    pub usage: Option<ProviderUsage>,
    pub inference_time_ms: Option<u64>,
    pub serialization_time_ns: Option<u64>,
    pub external_roundtrip_ns: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderAnswer {
    pub request_key: String,
    pub result: JudgmentResult,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProviderBatchResult {
    pub answers: Vec<ProviderAnswer>,
    pub metadata: ProviderMetadata,
}

pub fn validate_batch_result(
    requests: &[JudgmentRequest],
    response: &ProviderBatchResult,
) -> Result<(), JudgmentError> {
    let mut expected = HashSet::with_capacity(requests.len());
    for request in requests {
        if !expected.insert(request.request_key.as_str()) {
            return Err(JudgmentError::invalid_response(
                "provider batch contains duplicate request keys",
            ));
        }
    }
    if response.answers.len() != requests.len() {
        return Err(JudgmentError::invalid_response(
            "provider batch answer count does not match the request count",
        ));
    }
    let mut seen = HashSet::with_capacity(response.answers.len());
    for answer in &response.answers {
        if !seen.insert(answer.request_key.as_str()) {
            return Err(JudgmentError::invalid_response(
                "provider batch contains duplicate answer keys",
            ));
        }
        let request = requests
            .iter()
            .find(|request| request.request_key == answer.request_key)
            .ok_or_else(|| {
                JudgmentError::invalid_response("provider batch contains an unknown answer key")
            })?;
        match (&request.kind, &answer.result) {
            (QuestionKind::Noul, JudgmentResult::Noul(probability)) => {
                validate_probability(*probability)?;
            }
            (QuestionKind::Choice, JudgmentResult::Choice(label)) => {
                validate_label(label, &request.choices)?;
            }
            _ => {
                return Err(JudgmentError::invalid_response(
                    "provider batch answer type does not match the request",
                ));
            }
        }
    }
    if seen != expected {
        return Err(JudgmentError::invalid_response(
            "provider batch answer keys do not match the request keys",
        ));
    }
    Ok(())
}

/// Adapter interface consumed by the shared judgment executor.
pub trait Provider: Send + Sync {
    fn capabilities(&self) -> ProviderCapabilities;
    fn makes_external_request(&self) -> bool {
        false
    }
    fn judge(&self, request: &JudgmentRequest) -> Result<JudgmentResult, JudgmentError>;
    fn judge_with_metadata(
        &self,
        request: &JudgmentRequest,
    ) -> Result<ProviderBatchResult, JudgmentError> {
        Ok(ProviderBatchResult {
            answers: vec![ProviderAnswer {
                request_key: request.request_key.clone(),
                result: self.judge(request)?,
            }],
            metadata: ProviderMetadata::default(),
        })
    }
    fn cache_model_identity(&self, _requested_model: &str) -> Option<String> {
        None
    }
    fn estimate_request_bytes(&self, requests: &[JudgmentRequest]) -> usize {
        requests.iter().fold(1024usize, |total, request| {
            let text_bytes = request
                .criterion
                .as_ref()
                .map_or(0, String::len)
                .saturating_add(request.question.as_ref().map_or(0, String::len))
                .saturating_add(request.choices.iter().map(String::len).sum::<usize>());
            total
                .saturating_add(512)
                .saturating_add(request.state.canonical_bytes().len().saturating_mul(6))
                .saturating_add(text_bytes.saturating_mul(6))
                .saturating_add(request.request_key.len().saturating_mul(6))
        })
    }
    fn judge_many(
        &self,
        requests: &[JudgmentRequest],
    ) -> Result<ProviderBatchResult, JudgmentError> {
        if requests.len() != 1 {
            return Err(JudgmentError::configuration(
                "provider adapter does not implement multi-judgment requests",
            ));
        }
        let request = &requests[0];
        if self.capabilities().supports(request.kind) != CapabilityStatus::Verified {
            return Err(JudgmentError::configuration(
                "provider capability for this question type is not verified",
            ));
        }
        let response = ProviderBatchResult {
            answers: vec![ProviderAnswer {
                request_key: request.request_key.clone(),
                result: self.judge(request)?,
            }],
            metadata: ProviderMetadata::default(),
        };
        validate_batch_result(requests, &response)?;
        Ok(response)
    }
}
