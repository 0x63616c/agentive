use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRetryAdvice {
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorKind {
    Timeout,
    Authentication,
    RateLimited,
    Transport,
    InvalidRequest,
    Unavailable,
    Protocol,
    Unknown,
}

#[derive(Debug, Clone, Error, Serialize, Deserialize)]
#[error("provider error: {kind:?}: {message}")]
pub struct ProviderError {
    pub kind: ProviderErrorKind,
    pub message: String,
    pub retry_advice: ProviderRetryAdvice,
}

impl ProviderError {
    pub fn new(kind: ProviderErrorKind, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            kind,
            message: message.into(),
            retry_advice: ProviderRetryAdvice { retryable },
        }
    }

    pub fn retryable(kind: ProviderErrorKind, message: impl Into<String>) -> Self {
        Self::new(kind, message, true)
    }

    pub fn terminal(kind: ProviderErrorKind, message: impl Into<String>) -> Self {
        Self::new(kind, message, false)
    }

    pub fn is_retryable(&self) -> bool {
        self.retry_advice.retryable
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl ToolError {
    pub fn terminal(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
        }
    }

    pub fn retryable(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: true,
        }
    }
}

#[derive(Debug, Error)]
pub enum RunError {
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("duplicate tool name: {0}")]
    DuplicateToolName(String),
    #[error("context admission failed: {0}")]
    ContextLimit(String),
    #[error("provider protocol invalid: {0}")]
    ProviderProtocol(String),
    #[error("provider failed: {source}")]
    ProviderFailed {
        attempts: u32,
        source: ProviderError,
    },
    #[error("provider execution failed: {0}")]
    Provider(String),
    #[error("run cancelled")]
    Cancelled,
    #[error("model-call limit reached: {attempt_limit}")]
    ModelCallLimit { attempt_limit: usize },
    #[error("tool execution failed: {0}")]
    Tool(String),
}
