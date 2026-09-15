use crate::ToolErrorCode;
use crate::model::ModelTokenUsage;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Provider guidance for retrying a failed request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRetryAdvice {
    /// Whether the same request may be retried.
    pub retryable: bool,
    /// Whether retry safety is known before the provider has produced output.
    ///
    /// Older serialized advice did not carry this distinction, so it defaults
    /// to false and therefore remains conservative when read back.
    #[serde(default)]
    pub safe_before_output: bool,
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
    /// Stable failure classification suitable for application policy.
    pub kind: ProviderErrorKind,
    /// Model-safe failure summary.
    pub message: String,
    /// Retry guidance without transport diagnostics.
    pub retry_advice: ProviderRetryAdvice,
    /// Provider-reported usage for this failed attempt, when safely available.
    #[serde(default)]
    usage: Option<Box<ModelTokenUsage>>,
    /// Private diagnostics retained for the application error chain and never serialized.
    #[source]
    #[serde(skip)]
    source: Option<std::sync::Arc<dyn std::error::Error + Send + Sync>>,
}

impl ProviderError {
    /// Creates an error with explicit retry guidance.
    pub fn new(kind: ProviderErrorKind, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            kind,
            message: message.into(),
            retry_advice: ProviderRetryAdvice {
                retryable,
                safe_before_output: retryable,
            },
            usage: None,
            source: None,
        }
    }

    /// Creates a retryable provider error.
    pub fn retryable(kind: ProviderErrorKind, message: impl Into<String>) -> Self {
        Self::new(kind, message, true)
    }

    /// Creates a terminal provider error.
    pub fn terminal(kind: ProviderErrorKind, message: impl Into<String>) -> Self {
        Self::new(kind, message, false)
    }

    /// Attaches provider-reported usage for this failed attempt.
    #[must_use]
    pub fn with_usage(mut self, usage: ModelTokenUsage) -> Self {
        self.usage = Some(Box::new(usage));
        self
    }

    /// Borrows provider-reported usage for this failed attempt, when available.
    #[must_use]
    pub fn usage(&self) -> Option<&ModelTokenUsage> {
        self.usage.as_deref()
    }

    /// Attaches private diagnostics without exposing them to models or durable payloads.
    #[must_use]
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(std::sync::Arc::new(source));
        self
    }

    /// Returns whether the request may be retried.
    pub fn is_retryable(&self) -> bool {
        self.retry_advice.retryable
    }

    /// Returns whether retry safety is known before output was produced.
    pub fn is_safe_before_output(&self) -> bool {
        self.retry_advice.safe_before_output
    }
}

/// A model-safe tool failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError {
    /// Stable application error code.
    pub code: ToolErrorCode,
    /// Model-safe failure summary.
    pub message: String,
    /// Whether this tool failure may be retried.
    pub retryable: bool,
    /// Private diagnostics never serialized or returned to the model.
    #[serde(skip)]
    source: Option<std::sync::Arc<dyn std::error::Error + Send + Sync>>,
}

impl ToolError {
    /// Creates a terminal tool failure.
    pub fn terminal(code: impl AsRef<str>, message: impl Into<String>) -> Self {
        Self {
            code: ToolErrorCode::parse(code.as_ref())
                .unwrap_or_else(|_| ToolErrorCode("invalid_tool_error_code".to_string())),
            message: message.into(),
            retryable: false,
            source: None,
        }
    }

    /// Creates a retryable tool failure.
    pub fn retryable(code: impl AsRef<str>, message: impl Into<String>) -> Self {
        Self {
            code: ToolErrorCode::parse(code.as_ref())
                .unwrap_or_else(|_| ToolErrorCode("invalid_tool_error_code".to_string())),
            message: message.into(),
            retryable: true,
            source: None,
        }
    }

    /// Attaches private application diagnostics without changing the safe tool result.
    #[must_use]
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(std::sync::Arc::new(source));
        self
    }

    /// Returns private diagnostics for application logging.
    pub fn source(&self) -> Option<&(dyn std::error::Error + Send + Sync + 'static)> {
        self.source.as_deref()
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
    #[error("structured output invalid: {0}")]
    /// The provider returned output that could not be decoded as the requested type.
    StructuredOutput(String),
}
