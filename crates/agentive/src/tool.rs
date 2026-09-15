//! Tool contracts, invocation context, and schema validation.

mod context;
mod schema;

use crate::errors::ToolError;
use crate::ids::ToolName;
use futures::future::BoxFuture;
use serde_json::Value;

pub use context::{
    CancellationReason, CancellationToken, ToolContext, ToolDecodeError, ToolInvocation,
};
pub use schema::decode_tool_call_args;

/// Structured model-visible result from a tool.
pub type ToolOutput = Value;
/// Future returned by an asynchronous tool invocation.
pub type ToolCallFuture<'call> = BoxFuture<'call, Result<ToolOutput, ToolError>>;

/// Published immutable schema for a tool.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolSchema {
    /// JSON Schema value.
    pub json: Value,
}

/// Discoverable metadata for a registered tool.
#[derive(Debug, Clone)]
pub struct ToolMetadata {
    /// Stable tool name.
    pub name: ToolName,
    /// Model-visible description.
    pub description: &'static str,
    /// Argument schema.
    pub schema: ToolSchema,
    /// Whether calls can be safely retried.
    pub idempotent: bool,
}

/// Provider-neutral tool contract.
pub trait Tool: Send + Sync {
    /// Stable portable tool name.
    fn name(&self) -> &ToolName;
    /// Model-visible description.
    fn description(&self) -> &'static str;
    /// Argument JSON Schema.
    fn schema_json(&self) -> &Value;
    /// Whether retrying an invocation is safe.
    fn idempotent(&self) -> bool {
        false
    }
    /// Whether independent calls can execute concurrently.
    fn parallel_safe(&self) -> bool {
        false
    }
    /// Maximum attempts for an idempotent retryable failure.
    fn max_attempts(&self) -> u8 {
        if self.idempotent() { 2 } else { 1 }
    }
    /// Metadata used for discovery and provider descriptor compilation.
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
    /// Execute one invocation.
    fn call<'call>(&'call self, context: &'call ToolContext, args: Value) -> ToolCallFuture<'call>;
}
