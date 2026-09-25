//! 确定性离线 mock provider：零网络依赖，供 demo 与单元测试。
//!
//! 与真实 adapter 走同一 `JudgmentRequest` → `JudgmentResult` 校验路径。

use crate::judgment::{
    validate_label, validate_probability, JudgmentError, JudgmentRequest, JudgmentResult,
    QuestionKind,
};
use crate::provider::{
    CapabilityStatus, Provider, ProviderAnswer, ProviderBatchResult, ProviderCapabilities,
    ProviderMetadata,
};

pub const MOCK_EFFECTIVE_MODEL: &str = "duckjeu-deterministic-mock-v1";

/// FNV-1a 64 位；仅用于确定性派生，不用于安全用途。
fn fnv1a(bytes: &[u8], seed: u64) -> u64 {
    let mut hash = seed;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 由 canonical state 与问题内容确定性派生。
pub fn derive_hash(request: &JudgmentRequest) -> u64 {
    let mut hash = fnv1a(request.state.canonical_bytes(), 0xcbf2_9ce4_8422_2325);
    match request.kind {
        QuestionKind::Noul => {
            hash = fnv1a(b"noul", hash);
            if let Some(criterion) = &request.criterion {
                hash = fnv1a(criterion.as_bytes(), hash);
            }
        }
        QuestionKind::Choice => {
            hash = fnv1a(b"choice", hash);
            if let Some(question) = &request.question {
                hash = fnv1a(question.as_bytes(), hash);
            }
            for label in &request.choices {
                hash = fnv1a(b"\x1f", hash);
                hash = fnv1a(label.as_bytes(), hash);
            }
        }
    }
    hash
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MockProvider;

impl Provider for MockProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            protocol_version: "duckjeu-mock/1".to_string(),
            noul: CapabilityStatus::Verified,
            choice: CapabilityStatus::Verified,
            multi_state_batch: CapabilityStatus::Verified,
            max_batch_judgments: None,
            max_choices: None,
        }
    }

    fn judge(&self, request: &JudgmentRequest) -> Result<JudgmentResult, JudgmentError> {
        let hash = derive_hash(request);
        match request.kind {
            QuestionKind::Noul => {
                // 千分之一粒度、确定性、落在 [0,1]。
                let value = (hash % 1_000_001) as f64 / 1_000_000.0;
                Ok(JudgmentResult::Noul(validate_probability(value)?))
            }
            QuestionKind::Choice => {
                let index = (hash % request.choices.len() as u64) as usize;
                let label = request.choices[index].clone();
                Ok(JudgmentResult::Choice(validate_label(
                    &label,
                    &request.choices,
                )?))
            }
        }
    }

    fn judge_with_metadata(
        &self,
        request: &JudgmentRequest,
    ) -> Result<ProviderBatchResult, JudgmentError> {
        Ok(ProviderBatchResult {
            answers: vec![ProviderAnswer {
                request_key: request.request_key.clone(),
                result: self.judge(request)?,
            }],
            metadata: ProviderMetadata {
                effective_model: Some(MOCK_EFFECTIVE_MODEL.to_string()),
                ..ProviderMetadata::default()
            },
        })
    }

    fn cache_model_identity(&self, _requested_model: &str) -> Option<String> {
        Some(MOCK_EFFECTIVE_MODEL.to_string())
    }

    fn judge_many(
        &self,
        requests: &[JudgmentRequest],
    ) -> Result<ProviderBatchResult, JudgmentError> {
        let mut response = ProviderBatchResult {
            answers: Vec::with_capacity(requests.len()),
            metadata: ProviderMetadata {
                effective_model: Some(MOCK_EFFECTIVE_MODEL.to_string()),
                ..ProviderMetadata::default()
            },
        };
        for request in requests {
            response.answers.push(ProviderAnswer {
                request_key: request.request_key.clone(),
                result: self.judge(request)?,
            });
        }
        crate::provider::validate_batch_result(requests, &response)?;
        Ok(response)
    }
}
