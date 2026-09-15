//! Client-side Temporal run control over the durable Agentive workflow.

use agentive::AgentRunState;
use std::sync::{Arc, RwLock};
use temporalio_client::{
    Client, WorkflowCancelOptions, WorkflowGetResultOptions, WorkflowQueryOptions,
    WorkflowStartOptions,
    errors::{
        WorkflowGetResultError, WorkflowInteractionError, WorkflowQueryError, WorkflowStartError,
    },
};

use super::{
    AgentiveWorkflow, TemporalRunConfig, TemporalRunCursor, TemporalRunSnapshot,
    TemporalWorkflowInput,
};

/// Starts and reconnects to Agentive Temporal workflow executions.
#[derive(Clone, Debug)]
pub struct TemporalRuntime {
    client: Client,
}

impl TemporalRuntime {
    /// Creates a runtime using a client configured with the shared payload data converter.
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Starts one canonical Agentive run under its stable run identity.
    pub async fn start(
        &self,
        task_queue: impl Into<String>,
        state: AgentRunState,
        config: TemporalRunConfig,
    ) -> Result<TemporalRunHandle, WorkflowStartError> {
        let workflow_id = state.run_id.clone();
        self.client
            .start_workflow(
                AgentiveWorkflow::run,
                TemporalWorkflowInput::new(state, config),
                WorkflowStartOptions::new(task_queue, workflow_id.clone()).build(),
            )
            .await?;
        Ok(TemporalRunHandle {
            client: self.client.clone(),
            workflow_id,
            terminal_state: Arc::new(RwLock::new(None)),
        })
    }

    /// Reconnects to a run without starting another execution.
    #[must_use]
    pub fn reconnect(&self, workflow_id: impl Into<String>) -> TemporalRunHandle {
        TemporalRunHandle {
            client: self.client.clone(),
            workflow_id: workflow_id.into(),
            terminal_state: Arc::new(RwLock::new(None)),
        }
    }
}

/// A reconnectable handle for one Temporal-owned Agentive run.
#[derive(Clone, Debug)]
pub struct TemporalRunHandle {
    client: Client,
    workflow_id: String,
    terminal_state: Arc<RwLock<Option<AgentRunState>>>,
}

impl TemporalRunHandle {
    /// Returns the durable workflow identity.
    #[must_use]
    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }

    /// Requests cancellation; activity cancellation reaches Agentive's canonical token.
    pub async fn cancel(&self) -> Result<(), WorkflowInteractionError> {
        self.workflow_handle()
            .cancel(WorkflowCancelOptions::default())
            .await
    }

    /// Reads the latest committed state and its opaque progress cursor.
    pub async fn observe(&self) -> Result<Option<TemporalRunSnapshot>, WorkflowQueryError> {
        if let Ok(state) = self.terminal_state.read()
            && let Some(state) = state.as_ref()
        {
            return Ok(Some(TemporalRunSnapshot {
                state: state.clone(),
                cursor: TemporalRunCursor::terminal(),
            }));
        }
        self.workflow_handle()
            .query(
                AgentiveWorkflow::observe,
                (),
                WorkflowQueryOptions::default(),
            )
            .await
    }

    /// Waits for the terminal canonical Agentive result, following Continue-As-New runs.
    pub async fn result(&self) -> Result<AgentRunState, WorkflowGetResultError> {
        let state = self
            .workflow_handle()
            .get_result(WorkflowGetResultOptions::default())
            .await?;
        if let Ok(mut terminal_state) = self.terminal_state.write() {
            *terminal_state = Some(state.clone());
        }
        Ok(state)
    }

    fn workflow_handle(&self) -> temporalio_client::WorkflowHandle<Client, AgentiveWorkflow> {
        self.client
            .get_workflow_handle::<AgentiveWorkflow>(self.workflow_id.clone())
    }
}
