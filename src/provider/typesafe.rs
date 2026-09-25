//! TypeSafe System One HTTP adapter。
//!
//! 协议子集见 `specs/001-judgment-foundation/contracts/provider-protocol.md`。
//! 凭据只出现在 Authorization header；错误信息脱敏；不自动重试。

use std::collections::HashSet;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::judgment::{
    validate_label, validate_probability, JudgmentError, JudgmentRequest, JudgmentResult,
    QuestionKind,
};
use crate::provider::{
    validate_batch_result, CapabilityStatus, Provider, ProviderAnswer, ProviderBatchResult,
    ProviderCapabilities, ProviderMetadata, ProviderUsage,
};
use crate::serialize::TypedValue;

fn typed_answer_value<'a>(
    answer: &'a Value,
    expected_type: &str,
    value_field: &str,
) -> Result<&'a Value, JudgmentError> {
    let object = answer.as_object().ok_or_else(|| {
        JudgmentError::invalid_response("provider answer is not a typed answer object")
    })?;
    if object.get("type").and_then(Value::as_str) != Some(expected_type) {
        return Err(JudgmentError::invalid_response(format!(
            "provider answer type does not match the requested {expected_type} question"
        )));
    }
    object.get(value_field).ok_or_else(|| {
        JudgmentError::invalid_response(format!(
            "provider {expected_type} answer is missing the '{value_field}' field"
        ))
    })
}

const VERIFIED_BATCH_EFFECTIVE_MODEL: &str = "jev-1.13.0";

fn multi_state_batch_status(model: &str) -> CapabilityStatus {
    if matches!(model, "jev-latest" | VERIFIED_BATCH_EFFECTIVE_MODEL) {
        CapabilityStatus::Verified
    } else {
        CapabilityStatus::Unknown
    }
}

pub struct TypesafeProvider {
    agent: ureq::Agent,
    api_url: String,
    model: String,
    max_response_bytes: u64,
    credential: Option<String>,
    protocol_version: &'static str,
    multi_state_batch: CapabilityStatus,
    max_choices: Option<usize>,
    reject_truncated_header: bool,
    auth_hint: &'static str,
    cacheable_model_aliases: bool,
}

impl TypesafeProvider {
    pub fn new(
        api_url: String,
        model: String,
        timeout_ms: u64,
        max_response_bytes: u64,
        credential: String,
    ) -> Self {
        let multi_state_batch = multi_state_batch_status(&model);
        Self::with_protocol(
            api_url,
            model,
            timeout_ms,
            max_response_bytes,
            Some(credential),
            "typesafe-systemone/v1",
            multi_state_batch,
            None,
            false,
            "check DUCKJEU_API_KEY or TYPESAFE_API_KEY",
            true,
        )
    }

    pub fn new_localjev(
        api_url: String,
        model: String,
        timeout_ms: u64,
        max_response_bytes: u64,
        credential: Option<String>,
    ) -> Self {
        Self::with_protocol(
            api_url,
            model,
            timeout_ms,
            max_response_bytes,
            credential,
            "local-jev/64a0b31",
            CapabilityStatus::Unsupported,
            Some(255),
            true,
            "check DUCKJEU_LOCAL_JEV_API_KEY",
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with_protocol(
        api_url: String,
        model: String,
        timeout_ms: u64,
        max_response_bytes: u64,
        credential: Option<String>,
        protocol_version: &'static str,
        multi_state_batch: CapabilityStatus,
        max_choices: Option<usize>,
        reject_truncated_header: bool,
        auth_hint: &'static str,
        cacheable_model_aliases: bool,
    ) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(timeout_ms)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        TypesafeProvider {
            agent,
            api_url,
            model,
            max_response_bytes,
            credential,
            protocol_version,
            multi_state_batch,
            max_choices,
            reject_truncated_header,
            auth_hint,
            cacheable_model_aliases,
        }
    }

    /// 构造请求体；noul condition 放入 state，choice criteria 放入问题对象。
    pub fn build_body(&self, request: &JudgmentRequest) -> Value {
        let row = Self::state_row(request);
        let mut state = Map::new();
        state.insert("rows".to_string(), Value::Array(vec![row]));
        let mut question = Map::new();
        let (question_type, instructions): (&str, String) = match request.kind {
            QuestionKind::Noul => {
                state.insert(
                    "condition".to_string(),
                    Value::String(request.criterion.clone().unwrap_or_default()),
                );
                (
                    "noul",
                    "Evaluate whether state.condition holds for state.rows[0]. \
                     Answer with the probability that it holds."
                        .to_string(),
                )
            }
            QuestionKind::Choice => {
                let mut criteria = Map::new();
                for label in &request.choices {
                    criteria.insert(label.clone(), Value::Null);
                }
                question.insert("criteria".to_string(), Value::Object(criteria));
                (
                    "choice",
                    format!(
                        "Caller question: {}\nSelect the single candidate label that best answers this question for state.rows[0].",
                        request.question.as_deref().unwrap_or_default()
                    ),
                )
            }
        };
        question.insert("type".to_string(), Value::String(question_type.to_string()));
        question.insert("instructions".to_string(), Value::String(instructions));
        let mut questions = Map::new();
        questions.insert(request.request_key.clone(), Value::Object(question));

        json!({
            "model": self.model,
            "state": Value::Object(state),
            "questions": Value::Object(questions),
        })
    }

    /// Construct a TypeSafe composite-state request for compatible judgments.
    pub fn build_batch_body(&self, requests: &[JudgmentRequest]) -> Result<Value, JudgmentError> {
        let first = requests.first().ok_or_else(|| {
            JudgmentError::configuration("TypeSafe batch must contain at least one judgment")
        })?;
        if requests.len() == 1 {
            return Ok(self.build_body(first));
        }

        let mut keys = HashSet::with_capacity(requests.len());
        for request in requests {
            self.check_request_capabilities(request)?;
            if !keys.insert(request.request_key.as_str()) {
                return Err(JudgmentError::configuration(
                    "TypeSafe batch question keys must be unique",
                ));
            }
            if request.kind != first.kind
                || request.criterion != first.criterion
                || request.question != first.question
                || request.choices != first.choices
            {
                return Err(JudgmentError::configuration(
                    "TypeSafe batch judgments must share one compatible question context",
                ));
            }
        }

        let mut state = Map::new();
        state.insert(
            "rows".to_string(),
            Value::Array(requests.iter().map(Self::state_row).collect()),
        );
        match first.kind {
            QuestionKind::Noul => {
                let condition = first.criterion.as_deref().ok_or_else(|| {
                    JudgmentError::configuration("TypeSafe noul batch is missing its criterion")
                })?;
                state.insert(
                    "condition".to_string(),
                    Value::String(condition.to_string()),
                );
            }
            QuestionKind::Choice => {}
        }

        let mut questions = Map::new();
        for (row_index, request) in requests.iter().enumerate() {
            let mut question = Map::new();
            let (question_type, instructions) = match request.kind {
                QuestionKind::Noul => (
                    "noul",
                    format!(
                        "Evaluate whether state.condition holds for state.rows[{row_index}]. Answer with the probability that it holds."
                    ),
                ),
                QuestionKind::Choice => {
                    let mut criteria = Map::new();
                    for label in &request.choices {
                        criteria.insert(label.clone(), Value::Null);
                    }
                    question.insert("criteria".to_string(), Value::Object(criteria));
                    let caller_question = request.question.as_deref().ok_or_else(|| {
                        JudgmentError::configuration(
                            "TypeSafe choice batch is missing its question",
                        )
                    })?;
                    (
                        "choice",
                        format!(
                            "Caller question: {caller_question}\nSelect the single candidate label that best answers this question for state.rows[{row_index}]."
                        ),
                    )
                }
            };
            question.insert("type".to_string(), Value::String(question_type.to_string()));
            question.insert("instructions".to_string(), Value::String(instructions));
            questions.insert(request.request_key.clone(), Value::Object(question));
        }

        Ok(json!({
            "model": self.model,
            "state": Value::Object(state),
            "questions": Value::Object(questions),
        }))
    }

    fn state_row(request: &JudgmentRequest) -> Value {
        match request.state.value() {
            TypedValue::Text(text) => Value::String(text.clone()),
            other => crate::serialize::to_json_value(other),
        }
    }

    /// 解析并严格校验响应；任何歧义都失败。
    pub fn parse_response(
        &self,
        request: &JudgmentRequest,
        body: &str,
    ) -> Result<JudgmentResult, JudgmentError> {
        let mut response = self.parse_batch_response(std::slice::from_ref(request), body)?;
        Ok(response.answers.remove(0).result)
    }

    pub fn parse_batch_response(
        &self,
        requests: &[JudgmentRequest],
        body: &str,
    ) -> Result<ProviderBatchResult, JudgmentError> {
        let parsed: Value = serde_json::from_str(body).map_err(|_| {
            JudgmentError::invalid_response("provider returned a body that is not valid JSON")
        })?;
        let answers = parsed
            .get("answers")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                JudgmentError::invalid_response("provider response is missing the 'answers' object")
            })?;
        if answers.len() != requests.len()
            || requests
                .iter()
                .any(|request| !answers.contains_key(&request.request_key))
        {
            return Err(JudgmentError::invalid_response(
                "provider response answers do not match the requested question keys",
            ));
        }

        let mut response = ProviderBatchResult {
            answers: Vec::with_capacity(requests.len()),
            metadata: ProviderMetadata {
                requested_model: Some(self.model.clone()),
                effective_model: parsed
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                usage: parsed
                    .get("usage")
                    .and_then(Value::as_object)
                    .map(|usage| ProviderUsage {
                        input_tokens: usage.get("input_tokens").and_then(Value::as_u64),
                        output_tokens: usage.get("output_tokens").and_then(Value::as_u64),
                    }),
                inference_time_ms: None,
                serialization_time_ns: None,
                external_roundtrip_ns: None,
            },
        };
        for request in requests {
            let answer = &answers[&request.request_key];
            let result = match request.kind {
                QuestionKind::Noul => {
                    let value = typed_answer_value(answer, "noul", "noul")?
                        .as_f64()
                        .ok_or_else(|| {
                            JudgmentError::invalid_response("provider noul answer is not a number")
                        })?;
                    JudgmentResult::Noul(validate_probability(value)?)
                }
                QuestionKind::Choice => {
                    let label = typed_answer_value(answer, "choice", "choice")?
                        .as_str()
                        .ok_or_else(|| {
                            JudgmentError::invalid_response(
                                "provider choice answer is not a string",
                            )
                        })?;
                    JudgmentResult::Choice(validate_label(label, &request.choices)?)
                }
            };
            response.answers.push(ProviderAnswer {
                request_key: request.request_key.clone(),
                result,
            });
        }
        validate_batch_result(requests, &response)?;
        Ok(response)
    }
}

impl Provider for TypesafeProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            protocol_version: self.protocol_version.to_string(),
            noul: CapabilityStatus::Verified,
            choice: CapabilityStatus::Verified,
            multi_state_batch: self.multi_state_batch,
            max_batch_judgments: None,
            max_choices: self.max_choices,
        }
    }

    fn makes_external_request(&self) -> bool {
        true
    }

    fn judge(&self, request: &JudgmentRequest) -> Result<JudgmentResult, JudgmentError> {
        Ok(self.post_single(request)?.answers.remove(0).result)
    }

    fn judge_with_metadata(
        &self,
        request: &JudgmentRequest,
    ) -> Result<ProviderBatchResult, JudgmentError> {
        self.post_single(request)
    }

    fn cache_model_identity(&self, requested_model: &str) -> Option<String> {
        if !self.cacheable_model_aliases {
            return None;
        }
        let version = requested_model.strip_prefix("jev-")?;
        let components: Vec<_> = version.split('.').collect();
        if requested_model.trim() != requested_model
            || components.len() < 2
            || components
                .iter()
                .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return None;
        }
        Some(requested_model.to_string())
    }

    fn estimate_request_bytes(&self, requests: &[JudgmentRequest]) -> usize {
        self.build_batch_body(requests)
            .ok()
            .and_then(|body| serde_json::to_vec(&body).ok())
            .map_or(usize::MAX, |body| body.len())
    }

    fn judge_many(
        &self,
        requests: &[JudgmentRequest],
    ) -> Result<ProviderBatchResult, JudgmentError> {
        if requests.len() == 1 {
            return self.post_single(&requests[0]);
        }
        if self.multi_state_batch != CapabilityStatus::Verified {
            let message = match self.multi_state_batch {
                CapabilityStatus::Unknown => {
                    "provider multi-state batch capability is not verified"
                }
                CapabilityStatus::Unsupported => {
                    "provider does not support cross-state batch requests"
                }
                CapabilityStatus::Verified => unreachable!(),
            };
            return Err(JudgmentError::configuration(message));
        }
        let serialization_started = Instant::now();
        let body = self.build_batch_body(requests)?;
        self.post_body(requests, body, serialization_started)
    }
}

impl TypesafeProvider {
    fn post_single(&self, request: &JudgmentRequest) -> Result<ProviderBatchResult, JudgmentError> {
        self.check_request_capabilities(request)?;
        let serialization_started = Instant::now();
        let body = self.build_body(request);
        self.post_body(std::slice::from_ref(request), body, serialization_started)
    }

    fn post_body(
        &self,
        requests: &[JudgmentRequest],
        body: Value,
        serialization_started: Instant,
    ) -> Result<ProviderBatchResult, JudgmentError> {
        let body = serde_json::to_vec(&body)
            .map_err(|_| JudgmentError::provider("provider request could not be serialized"))?;
        let serialization_time_ns = serialization_started
            .elapsed()
            .as_nanos()
            .min(u64::MAX as u128) as u64;
        let mut builder = self
            .agent
            .post(&self.api_url)
            .header("Content-Type", "application/json");
        if let Some(credential) = self.credential.as_deref() {
            builder = builder.header("Authorization", &format!("Bearer {credential}"));
        }
        let roundtrip_started = Instant::now();
        let mut response = builder.send(&body).map_err(|err| {
            // ureq 错误不含凭据；此处仍统一改写为固定文案。
            JudgmentError::provider(format!("provider request failed: {err}"))
        })?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            // 远端正文不进入错误信息：服务端可能回显凭据，SC-003 要求错误输出零泄漏。
            // 不读取错误正文，避免大小限制或正文读取失败遮蔽 HTTP 状态码。
            // 只保留状态码，并针对鉴权失败给出可操作提示。
            if matches!(status, 401 | 403) {
                return Err(JudgmentError::provider(format!(
                    "provider returned HTTP {status} ({}).",
                    self.auth_hint
                )));
            }
            return Err(JudgmentError::provider(format!(
                "provider returned HTTP {status}"
            )));
        }
        if self.reject_truncated_header
            && response
                .headers()
                .get("x-local-jev-truncated")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        {
            return Err(JudgmentError::provider(
                "local-jev truncated the state; refusing a result based on partial input",
            ));
        }
        let body_text = response
            .body_mut()
            .with_config()
            .limit(self.max_response_bytes)
            .read_to_string()
            .map_err(|err| {
                JudgmentError::provider(format!("provider response could not be read: {err}"))
            })?;
        let mut result = self.parse_batch_response(requests, &body_text)?;
        if requests.len() > 1
            && result.metadata.effective_model.as_deref() != Some(VERIFIED_BATCH_EFFECTIVE_MODEL)
        {
            return Err(JudgmentError::provider(
                "provider model does not match the verified multi-state batch model",
            ));
        }
        result.metadata.serialization_time_ns = Some(serialization_time_ns);
        result.metadata.external_roundtrip_ns =
            Some(roundtrip_started.elapsed().as_nanos().min(u64::MAX as u128) as u64);
        Ok(result)
    }

    fn check_request_capabilities(&self, request: &JudgmentRequest) -> Result<(), JudgmentError> {
        if self.max_choices.is_some_and(|maximum| {
            request.kind == QuestionKind::Choice && request.choices.len() > maximum
        }) {
            return Err(JudgmentError::configuration(
                "choice count exceeds the verified provider limit",
            ));
        }
        Ok(())
    }
}
