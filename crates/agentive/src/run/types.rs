use crate::message::Message;
use crate::model::{AllOrError, ModelOutputFormat, ModelRequest, ModelResponse, ModelTokenUsage};
use crate::usage::RunUsage;
use std::time::Duration;

/// The lifecycle state exposed for an executing run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RunStatus {
    /// The run has been created but has not started work.
    Pending,
    /// The run is executing provider or tool effects.
    Running,
    /// The run produced a terminal successful result.
    Completed,
    /// The run ended without a complete result because a budget was exhausted.
    Incomplete,
    /// The run ended with a terminal failure.
    Failed,
    /// The run was cancelled before completion.
    Cancelled,
}

/// Runtime configuration for one agent run.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Provider-specific model identifier for this run.
    pub model: Option<String>,
    /// Instructions scoped to this run.
    pub run_instructions: Option<String>,
    /// Prior canonical conversation messages.
    pub history: Vec<Message>,
    /// Maximum provider calls.
    pub model_call_limit: usize,
    /// Tokens reserved for the model output.
    pub output_token_reserve: u64,
    /// Provider output contract for this run.
    pub output_format: ModelOutputFormat,
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
    /// Current depth within a delegated child-run tree.
    #[doc(hidden)]
    pub delegation_depth: u8,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            model: None,
            run_instructions: None,
            history: Vec::new(),
            model_call_limit: 6,
            output_token_reserve: 256,
            output_format: ModelOutputFormat::Text,
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

/// One provider request retained for diagnostics, including all transport attempts.
#[derive(Debug, Clone)]
pub struct RunRecord {
    /// Zero-based logical model-call round.
    pub model_call: u32,
    /// Canonical request sent for this round.
    pub request: ModelRequest,
    /// Successful canonical response, absent for a terminal provider failure.
    pub response: Option<ModelResponse>,
    /// Safe terminal provider failure, absent for a successful response.
    pub error: Option<crate::ProviderError>,
    /// Every runtime-owned transport attempt for this logical model round.
    pub attempts: Vec<ProviderAttempt>,
    /// Total wall-clock time consumed by this logical model round.
    pub elapsed_ms: u64,
}

/// Usage reported by one provider attempt, including failed retries.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ProviderAttempt {
    /// One-based attempt number within the logical provider effect.
    pub attempt: u8,
    /// Provider-reported usage, absent when the provider could not report it.
    #[serde(default)]
    pub usage: Option<ModelTokenUsage>,
    /// Provider-derived prompt bound plus reserved output capacity for this attempt.
    #[serde(default)]
    pub reserved_tokens: u64,
}

/// Terminal result of a local run.
#[derive(Debug, Clone)]
pub struct RunResult {
    /// Opaque identity of this run.
    pub run_id: String,
    /// Terminal lifecycle status.
    pub status: RunStatus,
    /// Final text output, when the run completed with text.
    pub text: Option<String>,
    /// Canonical conversation history at termination.
    pub history: Vec<Message>,
    /// Aggregate usage for the run tree.
    pub usage: RunUsage,
    /// Provider-round diagnostics retained by the local runtime.
    pub records: Vec<RunRecord>,
    /// Model-safe terminal failure text, when the run failed.
    pub error: Option<String>,
}

/// A typed model output together with its complete run result and usage.
#[derive(Debug, Clone)]
pub struct StructuredRunResult<T> {
    /// Deserialized final value.
    pub value: T,
    /// Complete underlying run result.
    pub run: RunResult,
}

/// Observable lifecycle event emitted by a local run.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum RunEvent {
    /// An event from a child run, attributed without exposing its history.
    Child {
        /// Opaque identity of the child run.
        run_id: String,
        /// The child lifecycle event.
        event: Box<RunEvent>,
    },
    /// The run status changed.
    StatusChanged {
        /// The new lifecycle status.
        status: RunStatus,
    },
    /// A provider round started.
    ModelCallStarted {
        /// Zero-based model-call round.
        round: u32,
    },
    /// A provider-native ordered delta received during a streaming model call.
    ModelDelta {
        /// Provider round that produced the delta.
        round: u32,
        /// The native provider event, preserved without synthesis or reordering.
        event: crate::ModelStreamEvent,
    },
    /// A provider round completed.
    ModelCallCompleted {
        /// Zero-based model-call round.
        round: u32,
        /// Provider-reported usage, when available.
        usage: Option<ModelTokenUsage>,
    },
    /// A tool invocation started.
    ToolInvocationStarted {
        /// Registered tool name.
        name: crate::ids::ToolName,
        /// Provider call identifier for this invocation.
        call_id: String,
    },
    /// A tool invocation completed.
    ToolInvocationCompleted {
        /// Registered tool name.
        name: crate::ids::ToolName,
        /// Provider call identifier for this invocation.
        call_id: String,
        /// One-based execution attempt number.
        attempt: u8,
        /// Whether the invocation completed successfully.
        ok: bool,
    },
}

/// Canonical outcome produced by executing one runtime-owned effect.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEffectOutcome {
    /// A provider effect completed.
    Provider {
        /// Canonical provider response.
        response: ModelResponse,
        /// Every attempt made for this logical effect, in execution order.
        #[serde(default)]
        attempts: Vec<ProviderAttempt>,
    },
    /// A provider effect reached a terminal safe failure.
    ProviderFailed {
        /// Safe terminal error, without transport diagnostics.
        error: crate::ProviderError,
        /// Every attempt made for this logical effect, in execution order.
        #[serde(default)]
        attempts: Vec<ProviderAttempt>,
    },
    /// The provider-derived reservation would exceed the remaining hard token budget.
    ProviderBudgetExhausted {
        /// Tokens required for one transport attempt.
        required_tokens: u64,
    },
    /// A tool effect completed.
    Tool {
        /// Model-visible JSON tool result.
        output: serde_json::Value,
        /// Number of attempts made for the invocation.
        attempts: u8,
        /// Child aggregate usage, trusted only for a registered delegation tool.
        child_usage: Option<RunUsage>,
    },
    /// Parallel-safe tool outputs in provider order.
    ToolBatch {
        /// Results ordered by their corresponding provider tool calls.
        outputs: Vec<ToolEffectResult>,
    },
}

/// Final model-visible output and runtime attempt count for one tool invocation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolEffectResult {
    /// Model-visible JSON tool result.
    pub output: serde_json::Value,
    /// Number of attempts made for the invocation.
    pub attempts: u8,
    /// Trusted aggregate usage returned by a registered child agent.
    pub child_usage: Option<RunUsage>,
}
