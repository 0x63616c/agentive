use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct RunId(String);

impl RunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().as_simple().to_string())
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct ToolInvocationId(String);

impl ToolInvocationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().as_simple().to_string())
    }
}

impl Default for ToolInvocationId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct ProviderCallId(String);

impl ProviderCallId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().as_simple().to_string())
    }

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

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub struct ToolName(String);

#[derive(Debug, Error)]
pub enum ToolNameError {
    #[error("tool name must be 1..64 alphanumeric ascii with underscores or hyphens")]
    InvalidName,
}

impl ToolName {
    pub fn new_unchecked(value: impl Into<String>) -> Self {
        Self(value.into())
    }

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
