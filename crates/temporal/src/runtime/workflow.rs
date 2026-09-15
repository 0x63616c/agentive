//! Deterministic Temporal workflow that drives the canonical Agentive state machine.

use std::time::Duration;

use agentive::{AgentRunEffect, AgentRunState, StateTransitionError};
use temporalio_common::RetryPolicy;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{
    ActivityCancellationType, ActivityExecutionError, ActivityOptions, ApplicationFailure,
    ContinueAsNewOptions, WorkflowContext, WorkflowContextView, WorkflowResult,
    WorkflowTermination,
};

use super::{
    AgentiveActivities, EffectActivityInput, TemporalRunConfigError, TemporalRunCursor,
    TemporalRunSnapshot, TemporalWorkflowInput,
};

// Temporal requires every activity to have either a start-to-close or a
// schedule-to-close timeout. An Agentive run without a deadline therefore uses
// the largest duration representable by the persisted millisecond contract,
// rather than silently imposing an adapter-specific runtime limit.
const UNBOUNDED_EFFECT_SCHEDULE_TO_CLOSE_TIMEOUT: Duration = Duration::from_millis(u64::MAX);
const EFFECT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);
const DURABLE_SAFE_INFRASTRUCTURE_MAX_ATTEMPTS: u32 = 2;

/// A Temporal workflow with no provider, tool, filesystem, randomness, or wall-clock work.
#[workflow]
#[derive(Default)]
pub struct AgentiveWorkflow {
    snapshot: Option<TemporalRunSnapshot>,
}

#[workflow_methods]
impl AgentiveWorkflow {
    /// Drives state through one Temporal activity for each explicit requested effect.
    #[allow(missing_docs)] // The Temporal macro generates an associated workflow constant.
    #[run]
    pub async fn run(
        context: &mut WorkflowContext<Self>,
        mut input: TemporalWorkflowInput,
    ) -> WorkflowResult<AgentRunState> {
        input.config.validate().map_err(config_failure)?;
        if !context.patched("agentive-temporal-runtime-v1") {
            return Err(WorkflowTermination::failed_application(
                ApplicationFailure::non_retryable(UnsupportedHistoryMarker),
            ));
        }
        update_snapshot(context, &input);
        loop {
            if input.state.finish_if_exhausted() {
                break;
            }
            let Some(effect) = input.state.next_effect() else {
                break;
            };
            if input.effects_in_execution >= input.config.continue_as_new_after_effects {
                input.effects_in_execution = 0;
                return match context.continue_as_new(input, ContinueAsNewOptions::default()) {
                    Ok(never) => match never {},
                    Err(termination) => Err(termination),
                };
            }
            input
                .config
                .validates_effect(&effect)
                .map_err(config_failure)?;
            let effect_id = effect_id(&effect).to_owned();
            let max_attempts = input
                .config
                .activity_max_attempts(&effect)
                .min(DURABLE_SAFE_INFRASTRUCTURE_MAX_ATTEMPTS);
            let activity = context
                .execute_activity(
                    AgentiveActivities::execute_effect,
                    EffectActivityInput {
                        state: input.state.clone(),
                        effect,
                    },
                    ActivityOptions::with_schedule_to_close_timeout(effect_deadline(&input.state))
                        .heartbeat_timeout(EFFECT_HEARTBEAT_TIMEOUT)
                        .cancellation_type(ActivityCancellationType::WaitCancellationCompleted)
                        // Application failures are marked non-retryable by the activity. This
                        // finite policy is solely for lost workers/completion acknowledgements.
                        .retry_policy(
                            RetryPolicy::builder()
                                .maximum_attempts(max_attempts)
                                .build(),
                        )
                        .build(),
                )
                .await;
            let outcome = match activity {
                Ok(outcome) => outcome,
                Err(ActivityExecutionError::Cancelled(_)) => {
                    input.state.cancel();
                    update_snapshot(context, &input);
                    return Ok(input.state);
                }
                Err(other) => return Err(WorkflowTermination::from(other)),
            };
            let (outcome, elapsed_ms) = outcome.into_parts();
            input
                .state
                .commit_effect(&effect_id, outcome)
                .map_err(state_failure)?;
            input.state.record_elapsed(elapsed_ms);
            input.state.finish_if_exhausted();
            input.completed_effects = input.completed_effects.saturating_add(1);
            input.effects_in_execution = input.effects_in_execution.saturating_add(1);
            update_snapshot(context, &input);
        }
        input.state.finish_if_exhausted();
        update_snapshot(context, &input);
        Ok(input.state)
    }

    /// Reads the latest committed canonical state without exposing Temporal internals.
    #[allow(missing_docs)] // The Temporal macro generates an associated query constant.
    #[query]
    pub fn observe(&self, _context: &WorkflowContextView) -> Option<TemporalRunSnapshot> {
        self.snapshot.clone()
    }
}

fn effect_deadline(state: &AgentRunState) -> Duration {
    state
        .plan
        .deadline_ms
        .map_or(UNBOUNDED_EFFECT_SCHEDULE_TO_CLOSE_TIMEOUT, |deadline| {
            Duration::from_millis(deadline.saturating_sub(state.elapsed_ms))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentive::AgentRunBudget;

    #[test]
    fn effect_deadline_uses_the_remaining_run_deadline() {
        let mut state = AgentRunState::new(
            "deadline",
            vec![],
            AgentRunBudget {
                model_call_limit: 1,
            },
        );
        state.plan.deadline_ms = Some(1_000);
        state.elapsed_ms = 250;

        assert_eq!(effect_deadline(&state), Duration::from_millis(750));
    }

    #[test]
    fn effect_deadline_does_not_apply_a_short_cap_without_a_run_deadline() {
        let state = AgentRunState::new(
            "unbounded",
            vec![],
            AgentRunBudget {
                model_call_limit: 1,
            },
        );

        assert!(effect_deadline(&state) > Duration::from_secs(60));
    }
}

fn update_snapshot(context: &mut WorkflowContext<AgentiveWorkflow>, input: &TemporalWorkflowInput) {
    context.state_mut(|workflow| {
        workflow.snapshot = Some(TemporalRunSnapshot {
            state: input.state.clone(),
            cursor: TemporalRunCursor::from_completed_effects(input.completed_effects),
        });
    });
}

fn effect_id(effect: &AgentRunEffect) -> &str {
    match effect {
        AgentRunEffect::ProviderCall { effect_id, .. }
        | AgentRunEffect::ToolCall { effect_id, .. }
        | AgentRunEffect::ToolBatch { effect_id, .. } => effect_id,
    }
}

fn state_failure(error: StateTransitionError) -> WorkflowTermination {
    WorkflowTermination::failed_application(ApplicationFailure::non_retryable(error))
}

fn config_failure(error: TemporalRunConfigError) -> WorkflowTermination {
    WorkflowTermination::failed_application(ApplicationFailure::non_retryable(error))
}

#[derive(Debug, thiserror::Error)]
#[error("workflow history predates the required agentive-temporal-runtime-v1 marker")]
struct UnsupportedHistoryMarker;
