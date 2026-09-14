//! Provider-neutral local agent primitives.
//!
//! This crate intentionally scopes to the local horizontal core in slice 1.

mod errors;
mod ids;
mod message;
mod model;
mod run;
mod tool;
mod usage;

pub use errors::{ProviderError, ProviderErrorKind, ProviderRetryAdvice, RunError, ToolError};
pub use ids::*;
pub use message::{
    CompiledRequest, InstructionFragment, Message, MessageContent, MessageRole, ToolCall,
    ToolResult,
};
pub use model::{
    ModelCapabilities, ModelFinishReason, ModelProvider, ModelRequest, ModelResponse,
    ModelStreamEvent, ModelTokenUsage, ModelToolCall, ProviderCallContext, ProviderToolDescriptor,
    UsageEstimator,
};
pub use run::{
    Agent, AgentBuilder, RunEvent, RunHandle, RunOptions, RunRecord, RunResult, RunStatus,
};
pub use tool::{
    decode_tool_call_args, CancellationReason, Tool, ToolCallFuture, ToolContext, ToolDecodeError,
    ToolInvocation, ToolMetadata, ToolOutput, ToolSchema,
};
pub use usage::{ModelCallUsage, RunUsage, TokenUsage};
