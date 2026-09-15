use super::{
    Agent, AgentEffectOutcome, AtomicRunStatus, EventBus, RunEvent, RunOptions, RunRecord,
    RunResult, RunStatus, emit_event, validate_history,
};
use crate::errors::RunError;
use crate::ids::RunId;
use crate::tool::CancellationToken;
use crate::{AgentRunBudget, AgentRunEffect, AgentRunPlan, AgentRunState, ToolRuntimePolicy};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(super) struct RunExecution {
    pub(super) agent: Agent,
    pub(super) run_id: RunId,
    pub(super) user_message: String,
    pub(super) options: RunOptions,
    pub(super) status: Arc<AtomicRunStatus>,
    pub(super) cancellation: Arc<AtomicBool>,
    pub(super) cancellation_token: CancellationToken,
    pub(super) events: EventBus,
}

pub(super) async fn execute_run(execution: RunExecution) -> Result<RunResult, RunError> {
    let RunExecution {
        agent,
        run_id,
        user_message,
        options,
        status,
        cancellation,
        cancellation_token,
        events,
    } = execution;
    let compiled = agent.compile_request(user_message, &options);
    validate_history(&compiled.messages)?;
    let plan = AgentRunPlan {
        instructions: compiled.instructions,
        tools: agent.tools.iter().map(policy_for).collect(),
        model: None,
        max_output_tokens: options.output_token_reserve,
        provider_max_attempts: options.provider_max_attempts.max(1),
        max_parallel_tool_calls: options.max_parallel_tool_calls.max(1),
        context_estimate: options.context_estimate,
        provider_enforced_limit_opt_out: options.provider_enforced_limit_opt_out,
        deadline_ms: options
            .deadline
            .map(|value| value.as_millis().try_into().unwrap_or(u64::MAX)),
        delegation_depth: options.delegation_depth,
        max_delegation_depth: options.max_delegation_depth,
        token_limit: options.token_limit,
    };
    let mut state = AgentRunState::with_plan(
        run_id.to_string(),
        compiled.messages,
        AgentRunBudget {
            model_call_limit: options.model_call_limit.try_into().unwrap_or(u32::MAX),
        },
        plan,
    );
    let mut records = Vec::new();
    let started = Instant::now();
    status.set(RunStatus::Running);
    emit_event(
        &events,
        RunEvent::StatusChanged {
            status: RunStatus::Running,
        },
    )
    .await;
    loop {
        if cancellation.load(Ordering::SeqCst) {
            return finish_cancelled(&mut state, &status, &events, records).await;
        }
        if state.finish_if_exhausted() {
            status.set(state.status);
            emit_event(
                &events,
                RunEvent::StatusChanged {
                    status: state.status,
                },
            )
            .await;
            return Ok(result_from_state(state, records));
        }
        let Some(effect) = state.next_effect() else {
            return Ok(result_from_state(state, records));
        };
        let id = effect_id(&effect);
        let effect_started = Instant::now();
        emit_started(&events, &effect).await;
        let outcome = match agent
            .execute_effect_with_observer(
                &state,
                effect.clone(),
                cancellation_token.clone(),
                events.observer(),
            )
            .await
        {
            Ok(value) => value,
            Err(RunError::Cancelled) => {
                return finish_cancelled(&mut state, &status, &events, records).await;
            }
            Err(error) => {
                state.fail(error.to_string());
                status.set(RunStatus::Failed);
                emit_event(
                    &events,
                    RunEvent::StatusChanged {
                        status: RunStatus::Failed,
                    },
                )
                .await;
                return Err(error);
            }
        };
        emit_completed(&events, &effect, &outcome).await;
        if let AgentRunEffect::ProviderCall { round, request, .. } = &effect {
            let AgentEffectOutcome::Provider { response } = &outcome else {
                return Err(RunError::ProviderProtocol(
                    "provider effect returned a tool outcome".into(),
                ));
            };
            records.push(RunRecord {
                model_call: *round,
                request: request.clone(),
                response: response.clone(),
                elapsed_ms: elapsed(effect_started),
            });
        }
        state.record_elapsed(elapsed(effect_started));
        state
            .commit_effect(&id, outcome)
            .map_err(|error| RunError::ProviderProtocol(error.to_string()))?;
        if !matches!(effect, AgentRunEffect::ProviderCall { .. }) {
            validate_history(&state.history)?;
        }
        if started.elapsed() >= options.deadline.unwrap_or(Duration::MAX) {
            return finish_cancelled(&mut state, &status, &events, records).await;
        }
        if let Some(text) = state.text.clone() {
            status.set(RunStatus::Completed);
            emit_event(
                &events,
                RunEvent::StatusChanged {
                    status: RunStatus::Completed,
                },
            )
            .await;
            return Ok(RunResult {
                run_id: state.run_id.clone(),
                status: RunStatus::Completed,
                text: Some(text),
                history: state.history,
                usage: state.usage,
                records,
                error: None,
            });
        }
    }
}

fn policy_for(registered: &crate::run::agent::RegisteredTool) -> ToolRuntimePolicy {
    let tool = registered.tool();
    ToolRuntimePolicy {
        descriptor: crate::ProviderToolDescriptor {
            name: tool.name().clone(),
            description: tool.description().to_string(),
            schema: tool.schema_json().clone(),
            idempotent: tool.idempotent(),
        },
        idempotent: tool.idempotent(),
        max_attempts: tool.max_attempts(),
        parallel_safe: tool.parallel_safe(),
        delegation: registered.delegation().is_some(),
    }
}
async fn emit_started(events: &EventBus, effect: &AgentRunEffect) {
    match effect {
        AgentRunEffect::ProviderCall { round, .. } => {
            emit_event(events, RunEvent::ModelCallStarted { round: *round }).await
        }
        AgentRunEffect::ToolCall { invocation, .. } => {
            emit_event(
                events,
                RunEvent::ToolInvocationStarted {
                    name: invocation.tool_name.clone(),
                    call_id: invocation.invocation_id.to_string(),
                },
            )
            .await
        }
        AgentRunEffect::ToolBatch { calls, .. } => {
            for call in calls {
                emit_event(
                    events,
                    RunEvent::ToolInvocationStarted {
                        name: call.invocation.tool_name.clone(),
                        call_id: call.invocation.invocation_id.to_string(),
                    },
                )
                .await
            }
        }
    }
}
async fn emit_completed(events: &EventBus, effect: &AgentRunEffect, outcome: &AgentEffectOutcome) {
    match (effect, outcome) {
        (AgentRunEffect::ProviderCall { round, .. }, AgentEffectOutcome::Provider { response }) => {
            emit_event(
                events,
                RunEvent::ModelCallCompleted {
                    round: *round,
                    usage: response.usage.clone(),
                },
            )
            .await
        }
        (
            AgentRunEffect::ToolCall { invocation, .. },
            AgentEffectOutcome::Tool {
                output, attempts, ..
            },
        ) => emit_tool_completed(events, invocation, *attempts, output).await,
        (AgentRunEffect::ToolBatch { calls, .. }, AgentEffectOutcome::ToolBatch { outputs }) => {
            for (call, result) in calls.iter().zip(outputs) {
                emit_tool_completed(events, &call.invocation, result.attempts, &result.output).await
            }
        }
        _ => {}
    }
}
async fn emit_tool_completed(
    events: &EventBus,
    invocation: &crate::ToolInvocation,
    attempts: u8,
    output: &serde_json::Value,
) {
    emit_event(
        events,
        RunEvent::ToolInvocationCompleted {
            name: invocation.tool_name.clone(),
            call_id: invocation.invocation_id.to_string(),
            attempt: attempts,
            ok: output.get("error").is_none(),
        },
    )
    .await;
}
async fn finish_cancelled(
    state: &mut AgentRunState,
    status: &AtomicRunStatus,
    events: &EventBus,
    records: Vec<RunRecord>,
) -> Result<RunResult, RunError> {
    state.cancel();
    status.set(RunStatus::Cancelled);
    emit_event(
        events,
        RunEvent::StatusChanged {
            status: RunStatus::Cancelled,
        },
    )
    .await;
    Ok(result_from_state(state.clone(), records))
}
fn effect_id(effect: &AgentRunEffect) -> String {
    match effect {
        AgentRunEffect::ProviderCall { effect_id, .. }
        | AgentRunEffect::ToolCall { effect_id, .. }
        | AgentRunEffect::ToolBatch { effect_id, .. } => effect_id.clone(),
    }
}
fn elapsed(start: Instant) -> u64 {
    start.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}
pub(super) fn remaining_time(state: &AgentRunState) -> Option<Duration> {
    state
        .plan
        .deadline_ms
        .map(|value| Duration::from_millis(value.saturating_sub(state.elapsed_ms)))
}
fn result_from_state(state: AgentRunState, records: Vec<RunRecord>) -> RunResult {
    let error = match state.status {
        RunStatus::Incomplete => Some("model call limit reached".to_string()),
        RunStatus::Cancelled => Some("run was cancelled".to_string()),
        _ => state.error.clone(),
    };
    RunResult {
        run_id: state.run_id,
        status: state.status,
        text: state.text,
        history: state.history,
        usage: state.usage,
        records,
        error,
    }
}
