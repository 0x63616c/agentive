//! Explicit tool-backed sub-agent delegation.
//!
//! A [`DelegationTool`] is an ordinary [`crate::Tool`]. It owns the child agent
//! and returns a compact, model-visible result; a child's conversation and
//! diagnostics never become parent history.

use crate::{
    Agent, AgentEffectOutcome, CancellationReason, DelegationBudget, RunOptions, RunStatus, Tool,
    ToolContext, ToolDecodeError, ToolError, ToolName, decode_tool_call_args,
};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

pub(crate) struct DelegationExecution {
    pub(crate) remaining: Option<std::time::Duration>,
    pub(crate) cancellation: crate::CancellationToken,
    pub(crate) delegation: (u8, u8),
    pub(crate) max_parallel_tool_calls: usize,
    pub(crate) budget: Option<DelegationBudget>,
    pub(crate) observer: Option<crate::run::RunObserver>,
}

/// Model-visible arguments for a [`DelegationTool`].
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
struct DelegationRequest {
    /// A self-contained task for the delegated agent.
    #[schemars(description = "A self-contained task for the delegated agent.")]
    task: String,
}

/// Structured, model-visible summary of a delegated run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationResult {
    /// Configured diagnostic name of the child agent.
    pub agent: String,
    /// Opaque identity of the child run.
    pub run_id: String,
    /// Child terminal status.
    pub status: RunStatus,
    /// Child final text, when available.
    pub text: Option<String>,
    /// Child aggregate token usage, without its conversation or records.
    pub usage: crate::RunUsage,
}

/// An explicit delegation tool that runs one configured child agent.
///
/// The name and description are intentionally supplied by the application,
/// rather than inferred from the child. This is the exact contract the model
/// sees in the parent's tool registry.
#[derive(Clone)]
pub struct DelegationTool {
    name: ToolName,
    description: &'static str,
    child: Agent,
    parallel_safe: bool,
}

impl DelegationTool {
    /// Creates a model-visible delegation tool.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is not a portable tool name or
    /// `description` is blank.
    pub fn new(
        name: impl AsRef<str>,
        description: &'static str,
        child: Agent,
    ) -> Result<Self, ToolError> {
        let name = ToolName::parse(name.as_ref()).map_err(|_| {
            ToolError::terminal(
                "invalid_delegation_tool",
                "delegation tool name must be a portable tool name",
            )
        })?;
        if description.trim().is_empty() {
            return Err(ToolError::terminal(
                "invalid_delegation_tool",
                "delegation tool description must not be blank",
            ));
        }
        Ok(Self {
            name,
            description,
            child,
            parallel_safe: false,
        })
    }

    /// Returns the configured child agent.
    #[must_use]
    pub fn child(&self) -> &Agent {
        &self.child
    }

    /// Marks independent invocations safe for bounded parallel execution.
    #[must_use]
    pub fn with_parallel_execution(mut self) -> Self {
        self.parallel_safe = true;
        self
    }

    pub(crate) async fn execute(
        &self,
        args: Value,
        execution: DelegationExecution,
    ) -> Result<AgentEffectOutcome, crate::RunError> {
        let request = decode_tool_call_args::<DelegationRequest>(args)
            .map_err(|error| crate::RunError::ProviderProtocol(error.to_string()))?;
        if request.task.trim().is_empty() {
            return Ok(safe_error(
                "invalid_arguments",
                "delegation task must not be blank",
            ));
        }
        if execution.delegation.0 >= execution.delegation.1 {
            return Ok(safe_error(
                "delegation_depth_exceeded",
                "delegation depth limit reached",
            ));
        }
        let Some(budget) = execution.budget else {
            return Ok(safe_error(
                "delegation_budget_exceeded",
                "delegation budget is unavailable",
            ));
        };
        if budget.model_call_limit == 0 || budget.token_limit == Some(0) {
            return Ok(safe_error(
                "delegation_budget_exceeded",
                "delegation budget is exhausted",
            ));
        }
        let handle = self.child.start_with_observer(
            request.task,
            RunOptions {
                deadline: execution.remaining,
                model_call_limit: budget.model_call_limit.try_into().unwrap_or(usize::MAX),
                token_limit: budget.token_limit,
                max_parallel_tool_calls: execution.max_parallel_tool_calls,
                max_delegation_depth: execution.delegation.1,
                delegation_depth: execution.delegation.0.saturating_add(1),
                ..RunOptions::default()
            },
            execution.observer,
        );
        let child_run_id = handle.run_id().to_string();
        tokio::select! {
            child = handle.wait() => match child {
                Ok(result) if matches!(result.status, RunStatus::Completed | RunStatus::Incomplete) => {
                    let usage = result.usage.clone();
                    let output = DelegationResult {
                        agent: self.child.name().to_string(), run_id: child_run_id,
                        status: result.status, text: result.text, usage,
                    };
                    serde_json::to_value(output)
                        .map(|output| AgentEffectOutcome::Tool { output, attempts: 1, child_usage: Some(result.usage) })
                        .map_err(|_| crate::RunError::ProviderProtocol("delegation result could not be encoded".into()))
                }
                Ok(result) => Ok(safe_error_with_usage(
                    "delegation_failed",
                    "delegated agent failed",
                    result.usage,
                )),
                Err(_) => Ok(safe_error_with_usage(
                    "delegation_failed",
                    "delegated agent failed",
                    handle.usage_snapshot(),
                )),
            },
            _ = execution.cancellation.cancelled() => {
                handle.cancel_with(CancellationReason::Parent);
                let usage = handle
                    .wait()
                    .await
                    .map_or_else(|_| handle.usage_snapshot(), |result| result.usage);
                Ok(safe_error_with_usage(
                    "delegation_cancelled",
                    "delegation was cancelled",
                    usage,
                ))
            }
        }
    }
}

fn safe_error(code: &str, message: &str) -> AgentEffectOutcome {
    safe_error_with_optional_usage(code, message, None)
}

fn safe_error_with_usage(code: &str, message: &str, usage: crate::RunUsage) -> AgentEffectOutcome {
    safe_error_with_optional_usage(code, message, Some(usage))
}

fn safe_error_with_optional_usage(
    code: &str,
    message: &str,
    child_usage: Option<crate::RunUsage>,
) -> AgentEffectOutcome {
    AgentEffectOutcome::Tool {
        output: serde_json::json!({ "error": { "code": code, "message": message } }),
        attempts: 1,
        child_usage,
    }
}

impl Tool for DelegationTool {
    fn name(&self) -> &ToolName {
        &self.name
    }

    fn description(&self) -> &'static str {
        self.description
    }

    fn schema_json(&self) -> &Value {
        static SCHEMA: OnceLock<Value> = OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::to_value(schema_for!(DelegationRequest))
                .unwrap_or_else(|_| serde_json::json!({}))
        })
    }

    fn parallel_safe(&self) -> bool {
        self.parallel_safe
    }

    #[allow(clippy::manual_async_fn)]
    fn call<'call>(
        &'call self,
        context: &'call ToolContext,
        args: Value,
    ) -> impl std::future::Future<Output = Result<Value, ToolError>> + Send + 'call {
        async move {
            let request = decode_tool_call_args::<DelegationRequest>(args)
                .map_err(decode_error_to_tool_error)?;
            if request.task.trim().is_empty() {
                return Err(ToolError::terminal(
                    "invalid_arguments",
                    "delegation task must not be blank",
                ));
            }
            if context.delegation_depth() >= context.max_delegation_depth() {
                return Err(ToolError::terminal(
                    "delegation_depth_exceeded",
                    "delegation depth limit reached",
                ));
            }

            let handle = self.child.start(
                request.task,
                RunOptions {
                    deadline: context.remaining(),
                    max_delegation_depth: context.max_delegation_depth(),
                    delegation_depth: context.delegation_depth().saturating_add(1),
                    ..RunOptions::default()
                },
            );
            let child_run_id = handle.run_id().to_string();
            tokio::select! {
                child = handle.wait() => match child {
                    Ok(result) if matches!(result.status, RunStatus::Completed | RunStatus::Incomplete) => {
                        let output = DelegationResult {
                            agent: self.child.name().to_string(),
                            run_id: child_run_id,
                            status: result.status,
                            text: result.text,
                            usage: result.usage,
                        };
                        serde_json::to_value(output).map_err(|_| ToolError::terminal(
                            "delegation_serialization_failed",
                            "delegation result could not be encoded",
                        ))
                    }
                    Ok(_) => Err(ToolError::terminal(
                        "delegation_failed",
                        "delegated agent did not complete successfully",
                    )),
                    Err(_) => Err(ToolError::terminal(
                        "delegation_failed",
                        "delegated agent failed",
                    )),
                },
                _ = context.cancelled() => {
                    handle.cancel_with(CancellationReason::Parent);
                    Err(ToolError::terminal("delegation_cancelled", "delegation was cancelled"))
                }
            }
        }
    }
}

fn decode_error_to_tool_error(error: ToolDecodeError) -> ToolError {
    ToolError::terminal(
        "invalid_arguments",
        format!("invalid delegation arguments: {error}"),
    )
}
