use super::{
    Agent, AgentEffectOutcome, RunObserver, ToolEffectResult, execute_provider_effect,
    remaining_time, validate_context,
};
use crate::errors::RunError;
use crate::model::{ProviderCallContext, UsageEstimator};
use crate::tool::{CancellationToken, Tool, ToolContext, ToolInvocation};
use crate::{AgentRunEffect, AgentRunState, ToolRuntimePolicy};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct ToolRuntimeContext {
    remaining: Option<Duration>,
    cancellation: CancellationToken,
    delegation: (u8, u8),
    delegation_budget: Option<crate::DelegationBudget>,
    observer: Option<RunObserver>,
}

pub(super) async fn execute_effect(
    agent: &Agent,
    state: &AgentRunState,
    effect: AgentRunEffect,
    cancellation: CancellationToken,
    observer: Option<RunObserver>,
) -> Result<AgentEffectOutcome, RunError> {
    match effect {
        AgentRunEffect::ProviderCall {
            request,
            provider_call_id,
            round,
            ..
        } => execute_provider(agent, state, request, provider_call_id, round, cancellation).await,
        AgentRunEffect::ToolCall {
            invocation,
            arguments,
            policy,
            delegation_budget,
            ..
        } => {
            execute_one_tool(
                agent,
                invocation,
                arguments,
                policy,
                ToolRuntimeContext {
                    remaining: remaining_time(state),
                    cancellation,
                    delegation: (state.plan.delegation_depth, state.plan.max_delegation_depth),
                    delegation_budget,
                    observer,
                },
            )
            .await
        }
        AgentRunEffect::ToolBatch { calls, .. } => {
            let mut outputs = Vec::with_capacity(calls.len());
            for chunk in calls.chunks(state.plan.max_parallel_tool_calls.max(1)) {
                let pending = chunk.iter().map(|call| {
                    execute_one_tool(
                        agent,
                        call.invocation.clone(),
                        call.arguments.clone(),
                        call.policy.clone(),
                        ToolRuntimeContext {
                            remaining: remaining_time(state),
                            cancellation: cancellation.clone(),
                            delegation: (
                                state.plan.delegation_depth,
                                state.plan.max_delegation_depth,
                            ),
                            delegation_budget: call.delegation_budget,
                            observer: observer.clone(),
                        },
                    )
                });
                for outcome in futures::future::try_join_all(pending).await? {
                    let AgentEffectOutcome::Tool {
                        output,
                        attempts,
                        child_usage,
                    } = outcome
                    else {
                        return Err(RunError::ProviderProtocol(
                            "tool batch member returned a non-tool outcome".into(),
                        ));
                    };
                    outputs.push(ToolEffectResult {
                        output,
                        attempts,
                        child_usage,
                    });
                }
            }
            Ok(AgentEffectOutcome::ToolBatch { outputs })
        }
    }
}

async fn execute_provider(
    agent: &Agent,
    state: &AgentRunState,
    request: crate::ModelRequest,
    provider_call_id: String,
    round: u32,
    cancellation: CancellationToken,
) -> Result<AgentEffectOutcome, RunError> {
    validate_context(
        &agent.provider,
        state.plan.context_estimate,
        state.plan.provider_enforced_limit_opt_out,
        &request,
        &UsageEstimator,
    )?;
    let started = Instant::now();
    let mut attempt = 1;
    loop {
        let context = ProviderCallContext {
            provider_call_id: provider_call_id.clone(),
            model_round: round,
            attempt,
            remaining_time: remaining_time(state)
                .map(|value| value.saturating_sub(started.elapsed())),
            cancelled: cancellation.flag(),
            cancel_notifier: cancellation.notifier(),
        };
        let mut provider_attempt = Box::pin(execute_provider_effect(
            &agent.provider,
            request.clone(),
            &context,
        ));
        let outcome = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                // Give cancellation-aware transports a bounded chance to observe the notifier.
                let _ = tokio::time::timeout(Duration::from_millis(10), provider_attempt.as_mut()).await;
                return Err(RunError::Cancelled);
            }
            outcome = provider_attempt.as_mut() => outcome,
        };
        match outcome {
            Ok(_) if cancellation.is_cancelled() => return Err(RunError::Cancelled),
            Ok(response) => return Ok(AgentEffectOutcome::Provider { response }),
            Err(source) if source.is_retryable() && attempt < state.plan.provider_max_attempts => {
                tokio::time::sleep(retry_delay(&provider_call_id, attempt)).await;
                attempt = attempt.saturating_add(1);
            }
            Err(source) => {
                return Err(RunError::ProviderFailed {
                    attempts: u32::from(attempt),
                    source,
                });
            }
        }
    }
}

async fn execute_one_tool(
    agent: &Agent,
    invocation: ToolInvocation,
    arguments: serde_json::Value,
    policy: Option<ToolRuntimePolicy>,
    runtime: ToolRuntimeContext,
) -> Result<AgentEffectOutcome, RunError> {
    let Some(policy) = policy else {
        return Ok(not_found());
    };
    let Some(tool) = agent
        .tools
        .iter()
        .find(|tool| tool.tool().name() == &invocation.tool_name)
    else {
        return Ok(not_found());
    };
    if let Some(child_tool) = tool.delegation() {
        return child_tool
            .execute(
                arguments,
                crate::DelegationExecution {
                    remaining: runtime.remaining,
                    cancellation: runtime.cancellation,
                    delegation: runtime.delegation,
                    budget: runtime.delegation_budget,
                    observer: runtime.observer,
                },
            )
            .await;
    }
    let outcome = execute_tool_effect(
        tool.tool(),
        invocation,
        arguments,
        policy,
        runtime.remaining,
        runtime.cancellation,
        runtime.delegation,
    )
    .await?;
    Ok(outcome)
}

fn not_found() -> AgentEffectOutcome {
    AgentEffectOutcome::Tool {
        output: serde_json::json!({ "error": { "code": "tool_not_found", "message": "tool not found" } }),
        attempts: 0,
        child_usage: None,
    }
}

async fn execute_tool_effect(
    tool: &dyn Tool,
    invocation: ToolInvocation,
    arguments: serde_json::Value,
    policy: ToolRuntimePolicy,
    remaining: Option<Duration>,
    cancellation: CancellationToken,
    delegation: (u8, u8),
) -> Result<AgentEffectOutcome, RunError> {
    let started = Instant::now();
    let mut attempt = 1;
    loop {
        if cancellation.is_cancelled() {
            return Err(RunError::Cancelled);
        }
        let context = ToolContext::for_runtime_with_delegation_depth(
            invocation.clone(),
            attempt,
            remaining.map(|value| value.saturating_sub(started.elapsed())),
            cancellation.clone(),
            delegation.0,
            delegation.1,
        );
        match tool.call(&context, arguments.clone()).await {
            Ok(output) => {
                return Ok(AgentEffectOutcome::Tool {
                    output,
                    attempts: attempt,
                    child_usage: None,
                });
            }
            Err(error) if error.retryable && policy.idempotent && attempt < policy.max_attempts => {
                tokio::time::sleep(retry_delay(&invocation.invocation_id.to_string(), attempt))
                    .await;
                attempt = attempt.saturating_add(1);
            }
            Err(error) => {
                return Ok(AgentEffectOutcome::Tool {
                    output: serde_json::json!({ "error": { "code": error.code, "message": error.message } }),
                    attempts: attempt,
                    child_usage: None,
                });
            }
        }
    }
}

pub(super) fn retry_delay(identity: &str, attempt: u8) -> Duration {
    const BASE_MS: u64 = 5;
    const MAX_MS: u64 = 250;
    let ceiling = BASE_MS
        .saturating_mul(1_u64 << attempt.saturating_sub(1).min(5))
        .min(MAX_MS);
    let seed = identity.bytes().fold(u64::from(attempt), |hash, byte| {
        hash.wrapping_mul(1_099_511_628_211)
            .wrapping_add(u64::from(byte))
    });
    Duration::from_millis(seed % ceiling.saturating_add(1))
}
