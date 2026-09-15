use super::{
    Agent, AgentEffectOutcome, AtomicRunStatus, EventBus, RunEvent, RunOptions, RunRecord,
    RunResult, RunStatus, emit_event, validate_history,
};
use crate::errors::RunError;
use crate::ids::RunId;
use crate::tool::CancellationToken;
use crate::{AgentRunEffect, AgentRunState};
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
    pub(super) usage: Arc<std::sync::Mutex<crate::RunUsage>>,
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
        usage,
    } = execution;
    let compiled = agent.compile_request(user_message, &options);
    let mut state = agent.prepare_state(run_id.to_string(), compiled.messages, &options)?;
    let mut records = Vec::new();
    let started = Instant::now();
    tracing::info!(run_id = %run_id, agent = agent.name(), "agentive run started");
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
            return finish_cancelled(&mut state, &status, &events, &usage, records).await;
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
            return finish_with_usage(state, records, &usage);
        }
        let Some(effect) = state.next_effect() else {
            return finish_with_usage(state, records, &usage);
        };
        let id = effect_id(&effect);
        let effect_started = Instant::now();
        tracing::debug!(
            run_id = %state.run_id,
            effect_id = id,
            effect_kind = effect_kind(&effect),
            "agentive effect started"
        );
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
                return finish_cancelled(&mut state, &status, &events, &usage, records).await;
            }
            Err(error) => {
                state.fail(error.to_string());
                update_usage(&usage, &state.usage);
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
            match &outcome {
                AgentEffectOutcome::Provider { response, attempts } => records.push(RunRecord {
                    model_call: *round,
                    request: request.clone(),
                    response: Some(response.clone()),
                    error: None,
                    attempts: attempts.clone(),
                    elapsed_ms: elapsed(effect_started),
                }),
                AgentEffectOutcome::ProviderFailed { error, attempts } => {
                    records.push(RunRecord {
                        model_call: *round,
                        request: request.clone(),
                        response: None,
                        error: Some(error.clone()),
                        attempts: attempts.clone(),
                        elapsed_ms: elapsed(effect_started),
                    });
                }
                AgentEffectOutcome::ProviderBudgetExhausted { .. } => {}
                AgentEffectOutcome::Tool { .. } | AgentEffectOutcome::ToolBatch { .. } => {
                    return Err(RunError::ProviderProtocol(
                        "provider effect returned a tool outcome".into(),
                    ));
                }
            }
        }
        let terminal_provider_failure =
            matches!(outcome, AgentEffectOutcome::ProviderFailed { .. });
        state.record_elapsed(elapsed(effect_started));
        state
            .commit_effect(&id, outcome)
            .map_err(|error| RunError::ProviderProtocol(error.to_string()))?;
        tracing::debug!(run_id = %state.run_id, effect_id = id, "agentive effect committed");
        update_usage(&usage, &state.usage);
        if terminal_provider_failure {
            status.set(RunStatus::Failed);
            emit_event(
                &events,
                RunEvent::StatusChanged {
                    status: RunStatus::Failed,
                },
            )
            .await;
            return finish_with_usage(state, records, &usage);
        }
        if !matches!(effect, AgentRunEffect::ProviderCall { .. }) && state.pending_tools.is_empty()
        {
            validate_history(&state.history)?;
        }
        if started.elapsed() >= options.deadline.unwrap_or(Duration::MAX) {
            return finish_cancelled(&mut state, &status, &events, &usage, records).await;
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
            let result = RunResult {
                run_id: state.run_id.clone(),
                status: RunStatus::Completed,
                text: Some(text),
                history: state.history,
                usage: state.usage,
                records,
                error: None,
            };
            update_usage(&usage, &result.usage);
            return Ok(result);
        }
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
        (
            AgentRunEffect::ProviderCall { round, .. },
            AgentEffectOutcome::Provider { response, .. },
        ) => {
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
    usage: &Arc<std::sync::Mutex<crate::RunUsage>>,
    records: Vec<RunRecord>,
) -> Result<RunResult, RunError> {
    state.cancel();
    update_usage(usage, &state.usage);
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
fn finish_with_usage(
    state: AgentRunState,
    records: Vec<RunRecord>,
    usage: &Arc<std::sync::Mutex<crate::RunUsage>>,
) -> Result<RunResult, RunError> {
    let result = result_from_state(state, records);
    update_usage(usage, &result.usage);
    Ok(result)
}
fn update_usage(target: &Arc<std::sync::Mutex<crate::RunUsage>>, usage: &crate::RunUsage) {
    if let Ok(mut target) = target.lock() {
        *target = usage.clone();
    }
}
fn effect_id(effect: &AgentRunEffect) -> String {
    match effect {
        AgentRunEffect::ProviderCall { effect_id, .. }
        | AgentRunEffect::ToolCall { effect_id, .. }
        | AgentRunEffect::ToolBatch { effect_id, .. } => effect_id.clone(),
    }
}
fn effect_kind(effect: &AgentRunEffect) -> &'static str {
    match effect {
        AgentRunEffect::ProviderCall { .. } => "provider",
        AgentRunEffect::ToolCall { .. } => "tool",
        AgentRunEffect::ToolBatch { .. } => "tool_batch",
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
