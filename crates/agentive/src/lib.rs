//! Provider-neutral local agent primitives.
//!
//! This crate intentionally scopes to the local horizontal core in slice 1.
#![allow(missing_docs)] // Public API documentation is completed alongside the generated reference guide.

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
    CompiledInstructions, CompiledRequest, InstructionFragment, Message, MessageContent,
    MessageRole, ToolCall, ToolResult,
};
pub use model::{
    AllOrError, ModelCapabilities, ModelFinishReason, ModelProvider, ModelRequest, ModelResponse,
    ModelStreamEvent, ModelTokenUsage, ModelToolCall, ProviderCallContext, ProviderToolDescriptor,
    UsageEstimator,
};
pub use run::{
    Agent, AgentBuilder, AgentEffectOutcome, RunEvent, RunHandle, RunOptions, RunRecord, RunResult,
    RunStatus, ToolEffectResult,
};
pub use state::{
    AgentRunBudget, AgentRunEffect, AgentRunPlan, AgentRunState, DelegationBudget,
    StateTransitionError, ToolEffectCall, ToolRuntimePolicy,
};
pub(crate) use subagent::DelegationExecution;
pub use subagent::{DelegationResult, DelegationTool};
pub use tool::{
    CancellationReason, CancellationToken, Tool, ToolCallFuture, ToolContext, ToolDecodeError,
    ToolInvocation, ToolMetadata, ToolOutput, ToolSchema, decode_tool_call_args,
};
pub use usage::{ModelCallUsage, RunUsage, TokenUsage};
