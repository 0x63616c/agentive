//! Tool contracts, invocation context, and schema validation.

mod context;
mod schema;

use crate::errors::ToolError;
use crate::ids::ToolName;
use serde_json::Value;
use std::future::Future;
use std::sync::Arc;

pub use context::{
    CancellationReason, CancellationToken, ToolContext, ToolDecodeError, ToolInvocation,
};
pub use schema::{decode_tool_call_args, validate_tool_arguments, validate_tool_definition};

/// Structured model-visible result from a tool.
pub type ToolOutput = Value;

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

/// One authoritative model-facing tool declaration.
pub type ToolDefinition = ToolMetadata;

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
    fn metadata(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().clone(),
            description: self.description(),
            schema: ToolSchema {
                json: self.schema_json().clone(),
            },
            idempotent: self.idempotent(),
        }
    }
    /// Execute one invocation.
    fn call<'call>(
        &'call self,
        context: &'call ToolContext,
        args: Value,
    ) -> impl Future<Output = Result<ToolOutput, ToolError>> + Send + 'call;
}

/// Object-safe boundary used exclusively by the heterogeneous agent registry.
///
/// Public tool implementations use [`Tool`] directly. This adapter is where the
/// registry deliberately erases their concrete futures while preserving the
/// operation borrow lifetime.
pub(crate) trait ErasedTool: Send + Sync {
    fn name(&self) -> &ToolName;
    fn parallel_safe(&self) -> bool;
    fn max_attempts(&self) -> u8;
    fn metadata(&self) -> ToolDefinition;
    fn call<'call>(
        &'call self,
        context: &'call ToolContext,
        args: Value,
    ) -> futures::future::BoxFuture<'call, Result<ToolOutput, ToolError>>;
}

/// Type-erases a concrete [`Tool`] only when it enters a heterogeneous registry.
pub(crate) struct ToolRegistryAdapter<T> {
    tool: Arc<T>,
}

/// A cloneable, heterogeneous registry handle for a concrete [`Tool`].
///
/// Constructing a handle is the sole public transition from native tool
/// authoring to the dynamic registry boundary. The concrete tool's future is
/// boxed internally, while callers continue to await this handle normally.
#[derive(Clone)]
pub struct ToolHandle(Arc<dyn ErasedTool>);

impl ToolHandle {
    /// Adds a concrete tool to a heterogeneous registry boundary.
    #[must_use]
    pub fn new<T: Tool + 'static>(tool: T) -> Self {
        Self(Arc::new(ToolRegistryAdapter::new(tool)))
    }

    pub(crate) fn from_arc<T: Tool + 'static>(tool: Arc<T>) -> Self {
        Self(Arc::new(ToolRegistryAdapter::from_arc(tool)))
    }

    /// Stable portable tool name.
    pub fn name(&self) -> &ToolName {
        self.0.name()
    }

    /// Immutable tool definition published to model providers.
    pub fn metadata(&self) -> ToolDefinition {
        self.0.metadata()
    }

    pub(crate) fn parallel_safe(&self) -> bool {
        self.0.parallel_safe()
    }

    pub(crate) fn max_attempts(&self) -> u8 {
        self.0.max_attempts()
    }

    /// Executes one invocation through this heterogeneous registry handle.
    pub async fn call(&self, context: &ToolContext, args: Value) -> Result<ToolOutput, ToolError> {
        self.0.call(context, args).await
    }
}

impl<T> ToolRegistryAdapter<T> {
    pub(crate) fn new(tool: T) -> Self {
        Self {
            tool: Arc::new(tool),
        }
    }

    pub(crate) fn from_arc(tool: Arc<T>) -> Self {
        Self { tool }
    }
}

impl<T: Tool> ErasedTool for ToolRegistryAdapter<T> {
    fn name(&self) -> &ToolName {
        self.tool.name()
    }

    fn parallel_safe(&self) -> bool {
        self.tool.parallel_safe()
    }

    fn max_attempts(&self) -> u8 {
        self.tool.max_attempts()
    }

    fn metadata(&self) -> ToolDefinition {
        self.tool.metadata()
    }

    fn call<'call>(
        &'call self,
        context: &'call ToolContext,
        args: Value,
    ) -> futures::future::BoxFuture<'call, Result<ToolOutput, ToolError>> {
        Box::pin(self.tool.call(context, args))
    }
}
