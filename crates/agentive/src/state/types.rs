//! Serializable state-machine data contracts.

use crate::{
    AllOrError, CompiledInstructions, Message, ModelRequest, ModelToolCall, ProviderToolDescriptor,
    RunStatus, RunUsage, ToolInvocation, ToolName,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// A completion that does not match the current durable effect.
#[derive(Debug, Clone, Error, Serialize, Deserialize, PartialEq, Eq)]
pub enum StateTransitionError {
    /// A stale, duplicate, or out-of-order effect was completed.
    #[error("unexpected effect `{received}`; expected {expected:?}")]
    UnexpectedEffect {
        received: String,
        expected: Option<String>,
    },
}

/// Durable limits that constrain a run without relying on runtime-local state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRunBudget {
    /// Maximum provider calls started by this run.
    pub model_call_limit: u32,
}

/// Runtime policy snapshotted for a model-visible tool at run start.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolRuntimePolicy {
    /// The published model-facing descriptor.
    pub descriptor: ProviderToolDescriptor,
    /// Whether retrying a failed invocation is explicitly safe.
    pub idempotent: bool,
    /// Maximum attempts for one logical invocation.
    pub max_attempts: u8,
    /// Whether this tool may be scheduled in a bounded parallel batch.
    pub parallel_safe: bool,
    /// Whether this tool delegates to a registered child agent.
    #[serde(default)]
    pub delegation: bool,
}

/// Serializable configuration compiled once at run start.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentRunPlan {
    /// Canonical instruction layers for every provider call.
    pub instructions: CompiledInstructions,
    /// Immutable model-visible tool registry and execution policies.
    pub tools: Vec<ToolRuntimePolicy>,
    /// Explicit model selection, if configured.
    pub model: Option<String>,
    /// Output capacity reserved for each provider call.
    pub max_output_tokens: u64,
    /// Maximum attempts for one provider effect; one preserves the safe default.
    pub provider_max_attempts: u8,
    /// Maximum concurrent members of one explicitly parallel-safe batch.
    #[serde(default = "default_parallel_tool_calls")]
    pub max_parallel_tool_calls: usize,
    /// Required context-admission precision.
    pub context_estimate: AllOrError,
    /// Whether provider-side context enforcement is an explicit opt-out.
    pub provider_enforced_limit_opt_out: bool,
    /// Optional durable elapsed-time allowance.
    pub deadline_ms: Option<u64>,
    /// Current immutable position in the delegation tree.
    #[serde(default)]
    pub delegation_depth: u8,
    /// Maximum permitted delegation depth for the full tree.
    #[serde(default = "default_max_delegation_depth")]
    pub max_delegation_depth: u8,
    /// Optional hard token budget for this run tree.
    #[serde(default)]
    pub token_limit: Option<u64>,
}

impl AgentRunPlan {
    /// Finds a policy in the immutable registry snapshot.
    #[must_use]
    pub fn tool_policy(&self, name: &ToolName) -> Option<&ToolRuntimePolicy> {
        self.tools
            .iter()
            .find(|policy| &policy.descriptor.name == name)
    }
}

/// A deterministic, serializable snapshot of a run between effects.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentRunState {
    /// Stable run identity supplied by the owning runtime.
    pub run_id: String,
    /// Immutable request and execution configuration.
    pub plan: AgentRunPlan,
    /// Canonical committed conversation history.
    pub history: Vec<Message>,
    /// Aggregate usage committed from completed provider effects.
    pub usage: RunUsage,
    /// Limits carried with the state for replay-equivalent admission.
    pub budget: AgentRunBudget,
    /// Number of provider effects committed so far.
    pub model_calls: u32,
    /// Runtime-reported elapsed wall time consumed by completed effects.
    pub elapsed_ms: u64,
    /// Model calls reserved by child runs started from this state.
    #[serde(default)]
    pub reserved_child_model_calls: u32,
    /// Conservative tokens reserved by local calls and child runs.
    #[serde(default)]
    pub reserved_tokens: u64,
    /// Tool calls awaiting completion, in provider order.
    pub pending_tools: Vec<ModelToolCall>,
    /// Current terminal status, if any.
    pub status: RunStatus,
    /// Final text, when completed.
    pub text: Option<String>,
    /// Safe terminal failure text, if the run failed.
    pub error: Option<String>,
}

/// One explicit runtime-owned effect requested by [`AgentRunState`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentRunEffect {
    /// Ask the configured provider for the next model response.
    ProviderCall {
        /// Stable effect identity.
        effect_id: String,
        /// Stable provider-call identity.
        provider_call_id: String,
        /// Model round beginning at zero.
        round: u32,
        /// Exact canonical request for this effect.
        request: ModelRequest,
    },
    /// Invoke one tool with stable logical identity.
    ToolCall {
        effect_id: String,
        invocation: ToolInvocation,
        arguments: Value,
        /// Snapshotted retry/concurrency policy, if the model named a known tool.
        policy: Option<ToolRuntimePolicy>,
        /// Deterministic resource reservation for a registered child agent.
        #[serde(default)]
        delegation_budget: Option<DelegationBudget>,
    },
    /// Execute an explicitly parallel-safe group, committing its outputs in provider order.
    ToolBatch {
        /// Stable identity for the whole ordered batch.
        effect_id: String,
        /// Calls in the exact order emitted by the provider.
        calls: Vec<ToolEffectCall>,
    },
}

/// One member of a runtime-owned tool batch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolEffectCall {
    /// Stable logical invocation identity.
    pub invocation: ToolInvocation,
    /// Model-provided JSON arguments.
    pub arguments: Value,
    /// Snapshotted execution policy, absent for an unknown tool.
    pub policy: Option<ToolRuntimePolicy>,
    /// Deterministic resource reservation for a registered child agent.
    #[serde(default)]
    pub delegation_budget: Option<DelegationBudget>,
}

const fn default_parallel_tool_calls() -> usize {
    4
}
const fn default_max_delegation_depth() -> u8 {
    4
}

/// Durable model and token allowance granted to one child subtree.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegationBudget {
    /// Maximum provider calls available to the child subtree.
    pub model_call_limit: u32,
    /// Optional conservative token allowance for the child subtree.
    pub token_limit: Option<u64>,
}
