use super::{
    Agent, AgentEffectOutcome, RunObserver, ToolEffectResult, execute_provider_effect,
    remaining_time, validate_context, validate_effect_state_history, validate_model_response,
};
use crate::errors::RunError;
use crate::model::{ProviderCallContext, UsageEstimator};
use crate::tool::{CancellationToken, ToolContext, ToolHandle, ToolInvocation};
use crate::{AgentRunEffect, AgentRunState, ProviderAttempt, ToolRuntimePolicy};
use futures::FutureExt;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct ToolRuntimeContext {
    remaining: Option<Duration>,
    cancellation: CancellationToken,
    delegation: (u8, u8),
    max_parallel_tool_calls: usize,
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
    validate_effect_state_history(state)?;
    agent.validate_state_plan(state)?;
    if state.next_effect().as_ref() != Some(&effect) {
        return Err(RunError::ProviderProtocol(
            "effect does not match the state's next effect".into(),
        ));
    }
    match effect {
        AgentRunEffect::ProviderCall {
            request,
            provider_call_id,
            round,
            ..
        } => {
            execute_provider(
                agent,
                state,
                request,
                provider_call_id,
                round,
                cancellation,
                observer,
            )
            .await
        }
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
                    max_parallel_tool_calls: state.plan.max_parallel_tool_calls,
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
                            max_parallel_tool_calls: state.plan.max_parallel_tool_calls,
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
    observer: Option<RunObserver>,
) -> Result<AgentEffectOutcome, RunError> {
    let provider_prompt_tokens = validate_context(
        &agent.provider,
        state.plan.context_estimate,
        state.plan.provider_enforced_limit_opt_out,
        &request,
        &UsageEstimator,
    )?;
    let started = Instant::now();
    let mut attempt = 1;
    let mut attempts = Vec::new();
    let remaining_calls = state.budget.model_call_limit.saturating_sub(
        state
            .provider_calls_started
            .max(state.model_calls)
            .saturating_add(state.reserved_child_model_calls),
    );
    let reservation = if state.plan.token_limit.is_some() {
        let Some(prompt_tokens) = provider_prompt_tokens else {
            return Err(RunError::ContextLimit(
                "hard token budget requires a provider-specific token count or bound".to_string(),
            ));
        };
        prompt_tokens.saturating_add(request.max_output_tokens)
    } else {
        state.next_model_token_reservation()
    };
    if state
        .plan
        .token_limit
        .is_some_and(|limit| reservation > limit.saturating_sub(state.reserved_tokens))
    {
        return Ok(AgentEffectOutcome::ProviderBudgetExhausted {
            required_tokens: reservation,
        });
    }
    let token_attempts = state.plan.token_limit.map_or(u32::MAX, |limit| {
        let remaining = limit.saturating_sub(state.reserved_tokens);
        u32::try_from(remaining / reservation.max(1)).unwrap_or(u32::MAX)
    });
    let allowed_attempts = u32::from(state.plan.provider_max_attempts)
        .min(remaining_calls)
        .min(token_attempts);
    loop {
        tracing::debug!(
            run_id = %state.run_id,
            provider_call_id,
            round,
            attempt,
            "provider attempt started"
        );
        let context = ProviderCallContext {
            provider_call_id: provider_call_id.clone(),
            model_round: round,
            attempt,
            remaining_time: remaining_time(state)
                .map(|value| value.saturating_sub(started.elapsed())),
            cancelled: cancellation.flag(),
            cancel_notifier: cancellation.notifier(),
        };
        let visible_delta = Arc::new(AtomicBool::new(false));
        let mut provider_attempt = Box::pin(execute_provider_effect(
            &agent.provider,
            request.clone(),
            &context,
            round,
            observer.as_ref(),
            &visible_delta,
        ));
        let outcome = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                // Give cancellation-aware transports a bounded chance to observe the notifier.
                let _ = tokio::time::timeout(Duration::from_millis(10), provider_attempt.as_mut()).await;
                return Err(RunError::Cancelled);
            }
            () = deadline_sleep(context.remaining_time) => {
                cancellation.cancel(crate::CancellationReason::Timeout);
                let _ = tokio::time::timeout(Duration::from_millis(10), provider_attempt.as_mut()).await;
                return Err(RunError::Cancelled);
            }
            outcome = provider_attempt.as_mut() => outcome,
        };
        match outcome {
            Ok(_) if cancellation.is_cancelled() => return Err(RunError::Cancelled),
            Ok(response) => {
                validate_model_response(&response, &provider_call_id)?;
                attempts.push(ProviderAttempt {
                    attempt,
                    usage: response.usage.clone(),
                    reserved_tokens: reservation,
                });
                return Ok(AgentEffectOutcome::Provider { response, attempts });
            }
            Err(source)
                if source.is_retryable()
                    && source.is_safe_before_output()
                    && !visible_delta.load(Ordering::SeqCst)
                    && u32::from(attempt) < allowed_attempts =>
            {
                attempts.push(ProviderAttempt {
                    attempt,
                    usage: source.usage().cloned(),
                    reserved_tokens: reservation,
                });
                wait_for_retry(
                    retry_delay(&provider_call_id, attempt),
                    context.remaining_time,
                    &cancellation,
                )
                .await?;
                attempt = attempt.saturating_add(1);
            }
            Err(source) => {
                attempts.push(ProviderAttempt {
                    attempt,
                    usage: source.usage().cloned(),
                    reserved_tokens: reservation,
                });
                return Ok(AgentEffectOutcome::ProviderFailed {
                    error: source,
                    attempts,
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
    if crate::tool::validate_tool_arguments(&arguments, &policy.descriptor.schema).is_err() {
        return Ok(invalid_arguments());
    }
    if let Some(child_tool) = tool.delegation() {
        return child_tool
            .execute(
                arguments,
                crate::DelegationExecution {
                    remaining: runtime.remaining,
                    cancellation: runtime.cancellation,
                    delegation: runtime.delegation,
                    max_parallel_tool_calls: runtime.max_parallel_tool_calls,
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

fn invalid_arguments() -> AgentEffectOutcome {
    AgentEffectOutcome::Tool {
        output: serde_json::json!({ "error": { "code": "invalid_arguments", "message": "tool arguments do not match the declared schema" } }),
        attempts: 0,
        child_usage: None,
    }
}

async fn execute_tool_effect(
    tool: &ToolHandle,
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
        tracing::debug!(
            run_id = %invocation.run_id,
            invocation_id = %invocation.invocation_id,
            tool = %invocation.tool_name,
            attempt,
            "tool attempt started"
        );
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
        let future = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tool.call(&context, arguments.clone())
        })) {
            Ok(future) => future,
            Err(_) => return Ok(panicked_tool_outcome(attempt)),
        };
        let mut tool_attempt = Box::pin(std::panic::AssertUnwindSafe(future).catch_unwind());
        let outcome = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(RunError::Cancelled),
            () = deadline_sleep(context.remaining()) => {
                cancellation.cancel(crate::CancellationReason::Timeout);
                return Err(RunError::Cancelled);
            }
            outcome = tool_attempt.as_mut() => outcome,
        };
        match outcome {
            Err(_) => {
                return Ok(panicked_tool_outcome(attempt));
            }
            Ok(Ok(output)) => {
                return Ok(AgentEffectOutcome::Tool {
                    output,
                    attempts: attempt,
                    child_usage: None,
                });
            }
            Ok(Err(error))
                if error.retryable && policy.idempotent && attempt < policy.max_attempts =>
            {
                wait_for_retry(
                    retry_delay(&invocation.invocation_id.to_string(), attempt),
                    context.remaining(),
                    &cancellation,
                )
                .await?;
                attempt = attempt.saturating_add(1);
            }
            Ok(Err(error)) => {
                return Ok(AgentEffectOutcome::Tool {
                    output: serde_json::json!({ "error": { "code": error.code, "message": error.message } }),
                    attempts: attempt,
                    child_usage: None,
                });
            }
        }
    }
}

fn panicked_tool_outcome(attempts: u8) -> AgentEffectOutcome {
    AgentEffectOutcome::Tool {
        output: serde_json::json!({ "error": { "code": "tool_panicked", "message": "tool execution failed" } }),
        attempts,
        child_usage: None,
    }
}

async fn deadline_sleep(remaining: Option<Duration>) {
    match remaining {
        Some(remaining) => tokio::time::sleep(remaining).await,
        None => std::future::pending::<()>().await,
    }
}

async fn wait_for_retry(
    delay: Duration,
    remaining: Option<Duration>,
    cancellation: &CancellationToken,
) -> Result<(), RunError> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(RunError::Cancelled),
        () = deadline_sleep(remaining) => {
            cancellation.cancel(crate::CancellationReason::Timeout);
            Err(RunError::Cancelled)
        }
        () = tokio::time::sleep(delay) => Ok(()),
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
