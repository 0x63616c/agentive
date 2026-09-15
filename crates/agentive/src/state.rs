//! Serializable, deterministic state transitions for an agent run.
//!
//! Runtimes own effects; this module owns no provider, tool, clock, task, or storage handle.

mod commits;
mod effects;
mod types;

pub use types::{
    AgentRunBudget, AgentRunEffect, AgentRunPlan, AgentRunState, DelegationBudget,
    StateTransitionError, ToolEffectCall, ToolRuntimePolicy,
};
