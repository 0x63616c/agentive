//! Provider-neutral local agent primitives.
//!
//! This crate intentionally scopes to the local horizontal core in slice 1.
#![warn(missing_docs)]

mod errors;
mod ids;
mod message;
mod model;
mod run;
mod state;
mod subagent;
mod tool;
mod usage;

pub use agentive_macros::tool;

pub use errors::{ProviderError, ProviderErrorKind, ProviderRetryAdvice, RunError, ToolError};
pub use ids::*;
pub use message::{
    CompiledInstructions, CompiledRequest, ImageMediaType, ImageSource, ImageUrl, InlineImageBytes,
    InstructionFragment, Message, MessageContent, MessageError, MessageRole, ToolCall, ToolResult,
};
pub use model::{
    AllOrError, FallbackProvider, ModelCapabilities, ModelFinishReason, ModelOutputFormat,
    ModelProvider, ModelRequest, ModelResponse, ModelStream, ModelStreamEvent, ModelTokenUsage,
    ModelToolCall, ProviderCallContext, ProviderToolDescriptor, StructuredOutputSupport,
    UsageEstimator,
};
pub use run::{
    Agent, AgentBuilder, AgentEffectOutcome, ProviderAttempt, RunEvent, RunHandle, RunOptions,
    RunRecord, RunResult, RunStatus, StructuredRunResult, ToolEffectResult,
};
pub use state::{
    AgentRunBudget, AgentRunEffect, AgentRunPlan, AgentRunState, DelegationBudget,
    StateTransitionError, ToolEffectCall, ToolRuntimePolicy,
};
pub(crate) use subagent::DelegationExecution;
pub use subagent::{DelegationResult, DelegationTool};
pub use tool::{
    CancellationReason, CancellationToken, Tool, ToolContext, ToolDecodeError, ToolDefinition,
    ToolHandle, ToolInvocation, ToolMetadata, ToolOutput, ToolSchema, decode_tool_call_args,
    validate_tool_arguments, validate_tool_definition,
};
pub use usage::{ModelCallUsage, RunUsage, TokenUsage};
