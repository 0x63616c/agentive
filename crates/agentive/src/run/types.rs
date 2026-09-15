use crate::message::Message;
use crate::model::{AllOrError, ModelRequest, ModelResponse, ModelTokenUsage};
use crate::usage::RunUsage;
use std::time::Duration;

/// The lifecycle state exposed for an executing run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RunStatus {
    Pending,
    Running,
    Completed,
    Incomplete,
    Failed,
    Cancelled,
}

/// Runtime configuration for one agent run.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Instructions scoped to this run.
    pub run_instructions: Option<String>,
    /// Prior canonical conversation messages.
    pub history: Vec<Message>,
    /// Maximum provider calls.
    pub model_call_limit: usize,
    /// Tokens reserved for the model output.
    pub output_token_reserve: u64,
    /// Optional hard token budget for this run and all delegated children.
    pub token_limit: Option<u64>,
    /// Maximum attempts for a retryable provider effect; defaults to one.
    pub provider_max_attempts: u8,
    /// Maximum concurrent explicitly parallel-safe tool calls; defaults to four.
    pub max_parallel_tool_calls: usize,
    /// Context-estimation policy.
    pub context_estimate: AllOrError,
    /// Whether the provider guarantees context enforcement.
    pub provider_enforced_limit_opt_out: bool,
    /// Optional elapsed-time budget for the run.
    pub deadline: Option<Duration>,
    /// Maximum nested delegation depth, including the current parent at depth zero.
    pub max_delegation_depth: u8,
    #[doc(hidden)]
    pub delegation_depth: u8,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            run_instructions: None,
            history: Vec::new(),
            model_call_limit: 6,
            output_token_reserve: 256,
            token_limit: None,
            provider_max_attempts: 1,
            max_parallel_tool_calls: 4,
            context_estimate: AllOrError::Exact,
            provider_enforced_limit_opt_out: false,
            deadline: None,
            max_delegation_depth: 4,
            delegation_depth: 0,
        }
    }
}

/// One completed provider request retained for diagnostics.
#[derive(Debug, Clone)]
pub struct RunRecord {
    pub model_call: u32,
    pub request: ModelRequest,
    pub response: ModelResponse,
    pub elapsed_ms: u64,
}

/// Terminal result of a local run.
#[derive(Debug, Clone)]
pub struct RunResult {
    /// Opaque identity of this run.
    pub run_id: String,
    pub status: RunStatus,
    pub text: Option<String>,
    pub history: Vec<Message>,
    pub usage: RunUsage,
    pub records: Vec<RunRecord>,
    pub error: Option<String>,
}

/// Observable lifecycle event emitted by a local run.
#[derive(Debug, Clone)]
pub enum RunEvent {
    /// An event from a child run, attributed without exposing its history.
    Child {
        /// Opaque identity of the child run.
        run_id: String,
        /// The child lifecycle event.
        event: Box<RunEvent>,
    },
    /// The run status changed.
    StatusChanged { status: RunStatus },
    /// A provider round started.
    ModelCallStarted { round: u32 },
    /// A provider round completed.
    ModelCallCompleted {
        round: u32,
        usage: Option<ModelTokenUsage>,
    },
    /// A tool invocation started.
    ToolInvocationStarted {
        name: crate::ids::ToolName,
        call_id: String,
    },
    /// A tool invocation completed.
    ToolInvocationCompleted {
        name: crate::ids::ToolName,
        call_id: String,
        attempt: u8,
        ok: bool,
    },
}

/// Canonical outcome produced by executing one runtime-owned effect.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEffectOutcome {
    /// A provider effect completed.
    Provider { response: ModelResponse },
    /// A tool effect completed.
    Tool {
        output: serde_json::Value,
        attempts: u8,
        /// Child aggregate usage, trusted only for a registered delegation tool.
        child_usage: Option<RunUsage>,
    },
    /// Parallel-safe tool outputs in provider order.
    ToolBatch { outputs: Vec<ToolEffectResult> },
}

/// Final model-visible output and runtime attempt count for one tool invocation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolEffectResult {
    pub output: serde_json::Value,
    pub attempts: u8,
    /// Trusted aggregate usage returned by a registered child agent.
    pub child_usage: Option<RunUsage>,
}
