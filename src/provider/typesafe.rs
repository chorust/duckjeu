//! TypeSafe System One HTTP adapter。
//!
//! 协议子集见 `specs/001-judgment-foundation/contracts/provider-protocol.md`。
//! 凭据只出现在 Authorization header；错误信息脱敏；不自动重试。

use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::judgment::{
    validate_label, validate_probability, JudgmentError, JudgmentRequest, JudgmentResult,
    QuestionKind,
};
use crate::provider::Provider;
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

pub struct TypesafeProvider {
    api_url: String,
    model: String,
    timeout_ms: u64,
    max_response_bytes: u64,
    credential: String,
}

impl TypesafeProvider {
    pub fn new(
        api_url: String,
        model: String,
        timeout_ms: u64,
        max_response_bytes: u64,
        credential: String,
    ) -> Self {
        TypesafeProvider {
            api_url,
            model,
            timeout_ms,
            max_response_bytes,
            credential,
        }
    }

    /// 构造请求体；noul condition 放入 state，choice criteria 放入问题对象。
    pub fn build_body(&self, request: &JudgmentRequest) -> Value {
        let row = match request.state.value() {
            TypedValue::Text(text) => Value::String(text.clone()),
            other => crate::serialize::to_json_value(other),
        };
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

    /// 解析并严格校验响应；任何歧义都失败。
    pub fn parse_response(
        &self,
        request: &JudgmentRequest,
        body: &str,
    ) -> Result<JudgmentResult, JudgmentError> {
        let parsed: Value = serde_json::from_str(body).map_err(|_| {
            JudgmentError::invalid_response("provider returned a body that is not valid JSON")
        })?;
        let answers = parsed
            .get("answers")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                JudgmentError::invalid_response("provider response is missing the 'answers' object")
            })?;
        if answers.len() != 1 || !answers.contains_key(&request.request_key) {
            return Err(JudgmentError::invalid_response(
                "provider response answers do not match the requested question keys",
            ));
        }
        let answer = &answers[&request.request_key];
        match request.kind {
            QuestionKind::Noul => {
                let value = typed_answer_value(answer, "noul", "noul")?
                    .as_f64()
                    .ok_or_else(|| {
                        JudgmentError::invalid_response("provider noul answer is not a number")
                    })?;
                Ok(JudgmentResult::Noul(validate_probability(value)?))
            }
            QuestionKind::Choice => {
                let label = typed_answer_value(answer, "choice", "choice")?
                    .as_str()
                    .ok_or_else(|| {
                        JudgmentError::invalid_response("provider choice answer is not a string")
                    })?;
                Ok(JudgmentResult::Choice(validate_label(
                    label,
                    &request.choices,
                )?))
            }
        }
    }
}

impl Provider for TypesafeProvider {
    fn judge(&self, request: &JudgmentRequest) -> Result<JudgmentResult, JudgmentError> {
        let body = self.build_body(request);
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(self.timeout_ms)))
            .http_status_as_error(false)
            .build();
        let agent = config.new_agent();
        let mut response = agent
            .post(&self.api_url)
            .header("Authorization", &format!("Bearer {}", self.credential))
            .header("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|err| {
                // ureq 错误不含凭据；此处仍统一改写为固定文案。
                JudgmentError::provider(format!("provider request failed: {err}"))
            })?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            // 远端正文不进入错误信息：服务端可能回显凭据，SC-003 要求错误输出零泄漏。
            // 不读取错误正文，避免大小限制或正文读取失败遮蔽 HTTP 状态码。
            // 只保留状态码，并针对鉴权失败给出可操作提示。
            let hint = match status {
                401 | 403 => " (check DUCKJEU_API_KEY or TYPESAFE_API_KEY)",
                _ => "",
            };
            return Err(JudgmentError::provider(format!(
                "provider returned HTTP {status}{hint}"
            )));
        }
        let body_text = response
            .body_mut()
            .with_config()
            .limit(self.max_response_bytes)
            .read_to_string()
            .map_err(|err| {
                JudgmentError::provider(format!("provider response could not be read: {err}"))
            })?;
        self.parse_response(request, &body_text)
    }
}
