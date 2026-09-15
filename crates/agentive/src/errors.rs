use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Provider guidance for retrying a failed request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRetryAdvice {
    /// Whether the same request may be retried.
    pub retryable: bool,
}

/// Stable classification of provider failures.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorKind {
    /// Deadline elapsed.
    Timeout,
    /// Authentication failed.
    Authentication,
    /// Capacity was limited.
    RateLimited,
    /// Transport failed.
    Transport,
    /// Request was rejected.
    InvalidRequest,
    /// Provider is unavailable.
    Unavailable,
    /// Provider protocol was invalid.
    Protocol,
    /// Failure lacks a specific class.
    Unknown,
}

/// A safe, typed provider failure.
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
#[error("provider error: {kind:?}: {message}")]
pub struct ProviderError {
    pub kind: ProviderErrorKind,
    pub message: String,
    pub retry_advice: ProviderRetryAdvice,
}

impl ProviderError {
    /// Creates an error with explicit retry guidance.
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

/// A model-safe tool failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError {
    /// Stable application error code.
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl ToolError {
    /// Creates a terminal tool failure.
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

/// A terminal run failure.
#[derive(Debug, Clone, Error)]
pub enum RunError {
    #[error("invalid configuration: {0}")]
    /// Configuration is invalid.
    Config(String),
    #[error("duplicate tool name: {0}")]
    /// Registry contains duplicate name.
    DuplicateToolName(String),
    #[error("context admission failed: {0}")]
    /// Context admission failed.
    ContextLimit(String),
    #[error("provider capability requirement not met: {0}")]
    /// A request requires a feature the configured provider does not support.
    Capability(String),
    #[error("provider protocol invalid: {0}")]
    /// Provider or history violated protocol.
    ProviderProtocol(String),
    #[error("provider failed: {source}")]
    /// Provider failed after attempts.
    ProviderFailed {
        /// Attempt count.
        attempts: u32,
        /// Underlying failure.
        source: ProviderError,
    },
    #[error("provider execution failed: {0}")]
    /// Runtime provider execution failed.
    Provider(String),
    #[error("run cancelled")]
    /// Run was cancelled.
    Cancelled,
    #[error("model-call limit reached: {attempt_limit}")]
    /// Model-call limit was reached.
    ModelCallLimit {
        /// Configured maximum.
        attempt_limit: usize,
    },
    #[error("tool execution failed: {0}")]
    /// Tool execution failed.
    Tool(String),
}
