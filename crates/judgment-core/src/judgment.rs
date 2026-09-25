//! 判断契约：请求、响应校验与阈值规则（判断语义层，不依赖 DuckDB 或网络）。
//!
//! 见 `specs/001-judgment-foundation/data-model.md` 与
//! `specs/001-judgment-foundation/contracts/sql-api.md`。

use std::fmt;
use std::hash::Hash;

use crate::serialize::CanonicalState;

/// v0.1 每个请求承载单个问题，question key 固定为 `r0`。
/// v0.2 批量扩展为 `r{i}`，`request_key` 即预留的请求关联身份。
pub const DEFAULT_REQUEST_KEY: &str = "r0";

/// 问题类型：二元概率（noul）与限定类别（choice）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuestionKind {
    Noul,
    Choice,
}

/// Non-secret provider context that scopes an identity.
///
/// `credential_scope` is an opaque host-generated value, never a credential or credential hash.
/// When the service's effective model is unknown, `model_binding_scope` prevents an unresolved
/// alias from being reused outside the host's current execution context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgmentIdentityContext {
    pub provider_namespace: String,
    pub endpoint: String,
    pub requested_model: String,
    pub effective_model: Option<String>,
    pub credential_scope: u64,
    pub model_binding_scope: u64,
}

/// Complete semantic identity for a judgment. `Hash` only locates candidates; derived `Eq`
/// compares every field, including the canonical payload and service context.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JudgmentIdentity {
    pub canonical_version: u32,
    pub state_kind: crate::serialize::StateKind,
    pub canonical_bytes: Vec<u8>,
    pub question_kind: QuestionKind,
    pub criterion: Option<String>,
    pub question: Option<String>,
    pub choices: Vec<String>,
    pub provider_namespace: String,
    pub endpoint: String,
    pub requested_model: String,
    pub effective_model: Option<String>,
    pub credential_scope: u64,
    /// Present only when the response cannot pin a mutable model alias to a known version.
    pub unresolved_model_scope: Option<u64>,
}

impl JudgmentIdentity {
    pub fn new(request: &JudgmentRequest, context: JudgmentIdentityContext) -> Self {
        let unresolved_model_scope = context
            .effective_model
            .is_none()
            .then_some(context.model_binding_scope);
        Self {
            canonical_version: request.state.canonical_version(),
            state_kind: request.state.kind(),
            canonical_bytes: request.state.canonical_bytes().to_vec(),
            question_kind: request.kind,
            criterion: request.criterion.clone(),
            question: request.question.clone(),
            choices: request.choices.clone(),
            provider_namespace: context.provider_namespace,
            endpoint: context.endpoint,
            requested_model: context.requested_model,
            effective_model: context.effective_model,
            credential_scope: context.credential_scope,
            unresolved_model_scope,
        }
    }

    /// Whether two distinct identities may share one provider batch without changing the
    /// question, service, model, credential, or unresolved-alias execution scope.
    pub fn same_batch_context(&self, other: &Self) -> bool {
        self.canonical_version == other.canonical_version
            && self.question_kind == other.question_kind
            && self.criterion == other.criterion
            && self.question == other.question
            && self.choices == other.choices
            && self.provider_namespace == other.provider_namespace
            && self.endpoint == other.endpoint
            && self.requested_model == other.requested_model
            && self.effective_model == other.effective_model
            && self.credential_scope == other.credential_scope
            && self.unresolved_model_scope == other.unresolved_model_scope
    }
}

/// 一次对外请求的完整语义内容。
#[derive(Debug, Clone, PartialEq)]
pub struct JudgmentRequest {
    pub kind: QuestionKind,
    pub state: CanonicalState,
    /// Noul 专用标准文本。
    pub criterion: Option<String>,
    /// Choice 专用问题文本。
    pub question: Option<String>,
    /// Choice 专用候选标签（原样保留，不修剪）。
    pub choices: Vec<String>,
    /// 请求关联身份；v0.1 恒为 `r0`。
    pub request_key: String,
}

impl JudgmentRequest {
    /// 构造 noul 请求（`jev_prob` / `jev_bool` 共用）。
    pub fn noul(state: CanonicalState, criterion: &str) -> Result<Self, JudgmentError> {
        if is_blank(criterion) {
            return Err(JudgmentError::invalid_input(
                "criterion must not be empty or whitespace",
            ));
        }
        Ok(JudgmentRequest {
            kind: QuestionKind::Noul,
            state,
            criterion: Some(criterion.to_string()),
            question: None,
            choices: Vec::new(),
            request_key: DEFAULT_REQUEST_KEY.to_string(),
        })
    }

    /// 构造 choice 请求。
    pub fn choice(
        state: CanonicalState,
        question: &str,
        choices: &[String],
    ) -> Result<Self, JudgmentError> {
        if is_blank(question) {
            return Err(JudgmentError::invalid_input(
                "question must not be empty or whitespace",
            ));
        }
        if choices.len() < 2 {
            return Err(JudgmentError::invalid_input(
                "choices must contain at least two labels",
            ));
        }
        for (idx, label) in choices.iter().enumerate() {
            if is_blank(label) {
                return Err(JudgmentError::invalid_input(
                    "choices must not contain empty or whitespace labels",
                ));
            }
            if choices[..idx].contains(label) {
                return Err(JudgmentError::invalid_input(
                    "choices must not contain duplicate labels",
                ));
            }
        }
        Ok(JudgmentRequest {
            kind: QuestionKind::Choice,
            state,
            criterion: None,
            question: Some(question.to_string()),
            choices: choices.to_vec(),
            request_key: DEFAULT_REQUEST_KEY.to_string(),
        })
    }
}

/// 经校验的返回值，尚未映射到 SQL 类型。
#[derive(Debug, Clone, PartialEq)]
pub enum JudgmentResult {
    Noul(f64),
    Choice(String),
}

impl JudgmentResult {
    pub fn as_probability(&self) -> Option<f64> {
        match self {
            JudgmentResult::Noul(p) => Some(*p),
            JudgmentResult::Choice(_) => None,
        }
    }

    pub fn as_label(&self) -> Option<&str> {
        match self {
            JudgmentResult::Noul(_) => None,
            JudgmentResult::Choice(label) => Some(label.as_str()),
        }
    }
}

/// 概率必须为 [0,1] 内有限值。
pub fn validate_probability(value: f64) -> Result<f64, JudgmentError> {
    if !value.is_finite() {
        return Err(JudgmentError::invalid_response(
            "provider returned a non-finite probability",
        ));
    }
    if !(0.0..=1.0).contains(&value) {
        return Err(JudgmentError::invalid_response(
            "provider returned a probability outside [0,1]",
        ));
    }
    Ok(value)
}

/// 类别必须是候选集内的精确成员（不解释顺序或分数）。
pub fn validate_label(label: &str, choices: &[String]) -> Result<String, JudgmentError> {
    if choices.iter().any(|c| c == label) {
        Ok(label.to_string())
    } else {
        Err(JudgmentError::invalid_response(
            "provider returned a label outside the candidate set",
        ))
    }
}

/// Validates an adapter result again at the executor boundary before it is reused or written.
pub fn validate_judgment_result(
    request: &JudgmentRequest,
    result: &JudgmentResult,
) -> Result<(), JudgmentError> {
    match (&request.kind, result) {
        (QuestionKind::Noul, JudgmentResult::Noul(probability)) => {
            validate_probability(*probability)?;
            Ok(())
        }
        (QuestionKind::Choice, JudgmentResult::Choice(label)) => {
            validate_label(label, &request.choices)?;
            Ok(())
        }
        _ => Err(JudgmentError::invalid_response(
            "provider result type does not match the requested judgment",
        )),
    }
}

/// `jev_bool` 的固定阈值：`p >= 0.5`（含边界）为真。
pub fn bool_from_probability(probability: f64) -> bool {
    probability >= 0.5
}

fn is_blank(s: &str) -> bool {
    s.trim().is_empty()
}

/// 判断语义层错误；`message` 已经脱敏，可直接写入 SQL 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgmentError {
    kind: ErrorKind,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// 输入不合法（空白标准/问题、非法选项等）。
    InvalidInput,
    /// provider 返回的内容不合法。
    InvalidResponse,
    /// provider 调用失败（网络、鉴权、超时、协议错误）。
    Provider,
    /// 配置或凭据问题。
    Configuration,
}

impl JudgmentError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        JudgmentError {
            kind,
            message: message.into(),
        }
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, message)
    }

    pub fn invalid_response(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidResponse, message)
    }

    pub fn provider(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Provider, message)
    }

    pub fn configuration(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Configuration, message)
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for JudgmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for JudgmentError {}
