use crate::errors::ToolError;
use crate::ids::{IdempotencyKey, ToolInvocationId, ToolName};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;
use tokio::sync::RwLock;

pub type ToolOutput = Value;

pub type ToolCallFuture<'call> = BoxFuture<'call, Result<ToolOutput, ToolError>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInvocation {
    pub run_id: String,
    pub invocation_id: ToolInvocationId,
    pub provider_call_id: Option<String>,
    pub tool_name: ToolName,
    pub model_round: u32,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Debug, Clone)]
pub enum CancellationReason {
    User,
    Parent,
    Timeout,
    Runtime,
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    invocation: ToolInvocation,
    attempt: u8,
    remaining_time: Option<std::time::Duration>,
    cancelled: std::sync::Arc<AtomicBool>,
    cancelled_reason: std::sync::Arc<RwLock<Option<CancellationReason>>>,
}

impl ToolContext {
    pub(crate) fn new(
        invocation: ToolInvocation,
        attempt: u8,
        remaining_time: Option<std::time::Duration>,
        cancelled: std::sync::Arc<AtomicBool>,
    ) -> Self {
        Self {
            invocation,
            attempt,
            remaining_time,
            cancelled,
            cancelled_reason: std::sync::Arc::new(RwLock::new(None)),
        }
    }

    pub fn invocation(&self) -> &ToolInvocation {
        &self.invocation
    }

    pub fn attempt(&self) -> u8 {
        self.attempt
    }

    pub fn remaining(&self) -> Option<std::time::Duration> {
        self.remaining_time
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) -> CancellationReason {
        let reason = self.cancelled_reason.read().await;
        reason.clone().unwrap_or(CancellationReason::Runtime)
    }
}

impl ToolContext {
    pub(crate) async fn set_cancelled(&self, reason: CancellationReason) {
        *self.cancelled_reason.write().await = Some(reason);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub json: Value,
}

#[derive(Debug, Clone)]
pub struct ToolMetadata {
    pub name: ToolName,
    pub description: &'static str,
    pub schema: ToolSchema,
    pub idempotent: bool,
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &ToolName;
    fn description(&self) -> &'static str;
    fn schema_json(&self) -> &Value;
    fn idempotent(&self) -> bool {
        false
    }
    fn max_attempts(&self) -> u8 {
        if self.idempotent() {
            2
        } else {
            1
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().clone(),
            description: self.description(),
            schema: ToolSchema {
                json: self.schema_json().clone(),
            },
            idempotent: self.idempotent(),
        }
    }

    fn call<'call>(&'call self, context: &'call ToolContext, args: Value) -> ToolCallFuture<'call>;
}

pub fn decode_tool_call_args<T: serde::de::DeserializeOwned>(
    value: Value,
) -> Result<T, ToolDecodeError> {
    serde_json::from_value::<T>(value)
        .map_err(|err| ToolDecodeError::UnknownFields(err.to_string()))
}

#[derive(Debug, Error)]
pub enum ToolDecodeError {
    #[error("tool argument decode failed: {0}")]
    UnknownFields(String),
}
