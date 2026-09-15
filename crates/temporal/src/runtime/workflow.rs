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

const EFFECT_START_TO_CLOSE_TIMEOUT: Duration = Duration::from_secs(60);
const EFFECT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

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
        while let Some(effect) = input.state.next_effect() {
            if input.effects_in_execution >= input.config.continue_as_new_after_effects {
                input.effects_in_execution = 0;
                return match context.continue_as_new(input, ContinueAsNewOptions::default()) {
                    Ok(never) => match never {},
                    Err(termination) => Err(termination),
                };
            }
            let effect_id = effect_id(&effect).to_owned();
            let outcome = context
                .execute_activity(
                    AgentiveActivities::execute_effect,
                    EffectActivityInput {
                        state: input.state.clone(),
                        effect,
                    },
                    ActivityOptions::with_start_to_close_timeout(EFFECT_START_TO_CLOSE_TIMEOUT)
                        .heartbeat_timeout(EFFECT_HEARTBEAT_TIMEOUT)
                        .cancellation_type(ActivityCancellationType::WaitCancellationCompleted)
                        .retry_policy(RetryPolicy::builder().maximum_attempts(1).build())
                        .build(),
                )
                .await
                .map_err(|error| match error {
                    ActivityExecutionError::Cancelled(_) => WorkflowTermination::cancelled(),
                    other => WorkflowTermination::from(other),
                })?;
            input
                .state
                .commit_effect(&effect_id, outcome)
                .map_err(state_failure)?;
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
