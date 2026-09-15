use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;
use uuid::Uuid;

/// Opaque identity of one agent run.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct RunId(String);

impl RunId {
    /// Generates a fresh run identity.
    pub fn new() -> Self {
        Self(Uuid::new_v4().as_simple().to_string())
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

/// Stable identity of one logical tool invocation.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct ToolInvocationId(String);

impl ToolInvocationId {
    /// Generates a fresh invocation identity.
    pub fn new() -> Self {
        Self(Uuid::new_v4().as_simple().to_string())
    }

    /// Creates a stable invocation identity supplied by a durable runtime.
    #[must_use]
    pub fn from_stable(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl Default for ToolInvocationId {
    fn default() -> Self {
        Self::new()
    }
}

/// Caller-supplied deduplication key for a logical tool invocation.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Wraps a caller-defined stable key.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

/// Identity of one provider-call attempt.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct ProviderCallId(String);

impl ProviderCallId {
    /// Generates a fresh provider-call identity.
    pub fn new() -> Self {
        Self(Uuid::new_v4().as_simple().to_string())
    }

    /// Returns the canonical identifier text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ProviderCallId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ProviderCallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for ToolInvocationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Validated model-facing tool name.
#[derive(Clone, Debug, Serialize, Eq, PartialEq, Hash)]
pub struct ToolName(String);

/// Validated model-visible code for a tool failure.
#[derive(Clone, Debug, Serialize, Eq, PartialEq, Hash)]
pub struct ToolErrorCode(pub(crate) String);

/// Tool-error-code validation failure.
#[derive(Debug, Error)]
pub enum ToolErrorCodeError {
    #[error("tool error code must be 1..64 lowercase ascii letters, digits, or underscores")]
    /// Code is empty, too long, or contains unsupported characters.
    InvalidCode,
}

impl ToolErrorCode {
    /// Parses a stable portable model-visible tool error code.
    pub fn parse(value: impl Into<String>) -> Result<Self, ToolErrorCodeError> {
        let candidate = value.into();
        if candidate.is_empty()
            || candidate.len() > 64
            || !candidate
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        {
            return Err(ToolErrorCodeError::InvalidCode);
        }
        Ok(Self(candidate))
    }

    /// Returns the canonical identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ToolErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ToolErrorCode {
    type Err = ToolErrorCodeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for ToolErrorCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)
            .and_then(|value| Self::parse(value).map_err(serde::de::Error::custom))
    }
}

/// Tool-name validation failure.
#[derive(Debug, Error)]
pub enum ToolNameError {
    #[error("tool name must be 1..64 alphanumeric ascii with underscores or hyphens")]
    /// Name is empty, too long, or contains unsupported characters.
    InvalidName,
}

impl ToolName {
    /// Parses and validates a portable provider-facing tool name.
    pub fn parse(value: impl Into<String>) -> Result<Self, ToolNameError> {
        let candidate = value.into();
        if candidate.is_empty() || candidate.len() > 64 {
            return Err(ToolNameError::InvalidName);
        }
        let valid = candidate
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
        if !valid {
            return Err(ToolNameError::InvalidName);
        }
        Ok(Self(candidate))
    }

    /// Returns the validated provider-facing tool name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for ToolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ToolName {
    type Err = ToolNameError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for ToolName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)
            .and_then(|value| Self::parse(value).map_err(serde::de::Error::custom))
    }
}

#[cfg(test)]
mod tests {
    use super::ToolName;

    #[test]
    fn deserialize_rejects_nonportable_tool_names() {
        assert!(serde_json::from_str::<ToolName>(r#""not a tool""#).is_err());
        let parsed = serde_json::from_str::<ToolName>(r#""safe_name-1""#);
        assert!(parsed.is_ok());
        if let Ok(name) = parsed {
            assert_eq!(name.as_str(), "safe_name-1");
        }
    }
}
