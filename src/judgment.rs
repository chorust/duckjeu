//! 判断契约：请求、响应校验与阈值规则（判断语义层，不依赖 DuckDB 或网络）。
//!
//! 见 `specs/001-judgment-foundation/data-model.md` 与
//! `specs/001-judgment-foundation/contracts/sql-api.md`。

use std::fmt;

use crate::serialize::CanonicalState;

/// v0.1 每个请求承载单个问题，question key 固定为 `r0`。
/// v0.2 批量扩展为 `r{i}`，`request_key` 即预留的请求关联身份。
pub const DEFAULT_REQUEST_KEY: &str = "r0";

/// 问题类型：二元概率（noul）与限定类别（choice）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionKind {
    Noul,
    Choice,
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
