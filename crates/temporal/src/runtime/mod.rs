//! Temporal workflow and activity adapters over the canonical Agentive state machine.

mod activity;
mod config;
mod handle;
mod workflow;

pub use activity::{AgentiveActivities, EffectActivityInput, EffectActivityOutput};
pub use config::{
    ADAPTER_SCHEMA_VERSION, TemporalRunConfig, TemporalRunConfigError, TemporalRunCursor,
    TemporalRunSnapshot, TemporalWorkflowInput, WORKFLOW_FINGERPRINT, WorkflowFingerprint,
};
pub use handle::{TemporalRunHandle, TemporalRuntime};
pub use workflow::AgentiveWorkflow;

use temporalio_sdk::{WorkerOptions, WorkflowRegistrationError};

/// Registers the Agentive workflow and its paired activities on one task queue.
///
/// The caller supplies a Temporal client configured with [`crate::temporal_data_converter`]
/// before constructing the worker, so every client and worker payload shares one pipeline.
///
/// # Errors
/// Returns an error when the Temporal SDK rejects workflow registration.
pub fn worker_options(
    task_queue: impl Into<String>,
    activities: AgentiveActivities,
) -> Result<WorkerOptions, WorkflowRegistrationError> {
    Ok(WorkerOptions::new(task_queue)
        .register_workflow::<AgentiveWorkflow>()?
        .register_activities(activities)
        .build())
}
