//! Secret-safe classification for App Server failures.

use agentive::{ProviderError, ProviderErrorKind};
use serde_json::Value;

pub(crate) fn classify_server_error(message: &Value) -> ProviderError {
    let code = message.pointer("/error/code").and_then(Value::as_i64);
    let message = message
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if code == Some(-32001) {
        ProviderError::retryable(
            ProviderErrorKind::RateLimited,
            "codex app server is overloaded",
        )
    } else if message.contains("unauthorized") || message.contains("login") {
        ProviderError::terminal(ProviderErrorKind::Authentication, "codex login is required")
    } else {
        ProviderError::terminal(
            ProviderErrorKind::Protocol,
            "codex app server rejected the request",
        )
    }
}
pub(crate) fn classify_turn_failure(params: &Value) -> ProviderError {
    if params
        .pointer("/turn/error/codexErrorInfo/code")
        .and_then(Value::as_str)
        == Some("unauthorized")
    {
        ProviderError::terminal(ProviderErrorKind::Authentication, "codex login is required")
    } else {
        ProviderError::terminal(ProviderErrorKind::Unavailable, "codex turn failed")
    }
}
pub(crate) fn transport_error() -> ProviderError {
    ProviderError::retryable(
        ProviderErrorKind::Transport,
        "codex app server transport failed",
    )
}
pub(crate) fn protocol_error() -> ProviderError {
    ProviderError::terminal(
        ProviderErrorKind::Protocol,
        "unsupported codex app server protocol",
    )
}
pub(crate) fn cancelled_error() -> ProviderError {
    ProviderError::terminal(ProviderErrorKind::Timeout, "codex turn was cancelled")
}
pub(crate) fn phase_timeout_error() -> ProviderError {
    ProviderError::retryable(
        ProviderErrorKind::Timeout,
        "codex app server protocol phase timed out",
    )
}
