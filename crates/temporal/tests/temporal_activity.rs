//! Temporal SDK activity-environment coverage for the canonical durable effect seam.

#![allow(clippy::expect_used)] // Fixture assertions make failures legible.

use agentive::{
    Agent, AgentRunBudget, AgentRunState, Message, ModelCapabilities, ModelProvider, ModelRequest,
    ModelResponse, ProviderCallContext, RunStatus, Tool, ToolCallFuture, ToolContext, ToolName,
};
use agentive_temporal::runtime::{
    AgentiveActivities, AgentiveWorkflow, EffectActivityInput, TemporalRunConfig, TemporalRuntime,
};
use agentive_temporal::{
    FilesystemStorage, ObjectId, PayloadLimits, PayloadPipeline, StorageId, temporal_data_converter,
};
use agentive_test::ScriptedProvider;
use serde_json::{Value, json};
use temporalio_client::errors::WorkflowGetResultError;
use temporalio_client::{ClientOptions, WorkflowFetchHistoryOptions, WorkflowHistory};
use temporalio_sdk::testing::{
    ActivityEnvironment, LocalWorkflowEnvironmentOptions, WorkflowEnvironment,
};
use temporalio_sdk::{
    Runtime, Worker,
    workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions},
};

#[derive(Clone)]
struct ImageCapableScriptedProvider(ScriptedProvider);

struct NoopTool(ToolName);

impl NoopTool {
    fn new() -> Self {
        Self("noop".parse().expect("valid fixture tool name"))
    }
}

impl Tool for NoopTool {
    fn name(&self) -> &ToolName {
        &self.0
    }
    fn description(&self) -> &'static str {
        "completes exactly once"
    }
    fn schema_json(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| json!({"type":"object","additionalProperties":false}))
    }
    fn call<'a>(&'a self, _: &'a ToolContext, _: Value) -> ToolCallFuture<'a> {
        Box::pin(async { Ok(json!({"ok": true})) })
    }
}

impl ModelProvider for ImageCapableScriptedProvider {
    fn generate<'call>(
        &'call self,
        request: ModelRequest,
        context: &'call ProviderCallContext,
    ) -> impl std::future::Future<Output = Result<ModelResponse, agentive::ProviderError>> + Send + 'call
    {
        self.0.generate(request, context)
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            supports_tool_calls: true,
            supports_streaming: false,
            supports_images: true,
            supports_context_count_estimate: true,
            max_context_tokens: Some(1_000_000),
            exact_context_counting: true,
        }
    }
}

#[tokio::test]
async fn activity_environment_executes_and_commits_one_canonical_provider_effect()
-> Result<(), Box<dyn std::error::Error>> {
    let provider = ScriptedProvider::new().respond_with_text("durably done");
    let activities = AgentiveActivities::new(Agent::builder().provider(provider.clone()).build()?);
    let mut state = AgentRunState::new(
        "durable-activity",
        vec![],
        AgentRunBudget {
            model_call_limit: 1,
        },
    );
    let effect = state.next_effect().expect("initial provider effect");
    let effect_id = match &effect {
        agentive::AgentRunEffect::ProviderCall { effect_id, .. } => effect_id.clone(),
        agentive::AgentRunEffect::ToolCall { .. } | agentive::AgentRunEffect::ToolBatch { .. } => {
            unreachable!("initial effect is provider")
        }
    };

    let outcome = ActivityEnvironment::builder_with_default()
        .register_activities(activities)
        .build()
        .run(
            AgentiveActivities::execute_effect,
            EffectActivityInput {
                state: state.clone(),
                effect,
            },
        )
        .await?;

    state.commit_effect(&effect_id, outcome)?;
    assert_eq!(state.status, RunStatus::Completed);
    assert_eq!(state.text.as_deref(), Some("durably done"));
    provider.assert_finished();
    Ok(())
}

#[tokio::test]
#[ignore = "downloads and starts the official Temporal CLI dev server; run before release"]
async fn local_temporal_worker_runs_the_canonical_state_over_filesystem_payloads()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let storage_id = StorageId::new("local-e2e")?;
    let pipeline = PayloadPipeline::builder()
        .storage(
            storage_id.clone(),
            FilesystemStorage::new(directory.path())?,
        )?
        .default_storage(storage_id)
        .limits(PayloadLimits::new(32 * 1024 * 1024, 32 * 1024 * 1024)?)
        .build()?;
    let converter = temporal_data_converter(pipeline);
    let client_options = ClientOptions::new("default")
        .data_converter(converter)
        .build();
    let environment = WorkflowEnvironment::start_local(
        LocalWorkflowEnvironmentOptions::builder()
            .client_options(client_options)
            .build(),
    )
    .await?;
    let queue = format!("agentive-e2e-{}", ObjectId::random().to_hex());
    let scripted = ScriptedProvider::new()
        .respond_with_tool_calls(vec![("noop", json!({}))])
        .respond_with_text("local durable result");
    let provider = ImageCapableScriptedProvider(scripted.clone());
    let options = agentive_temporal::runtime::worker_options(
        queue.clone(),
        AgentiveActivities::new(
            Agent::builder()
                .provider(provider.clone())
                .tool(NoopTool::new())
                .build()?,
        ),
    )?;
    let runtime = Runtime::from_current_tokio(Default::default())?;
    let mut worker = Worker::new(&runtime, environment.client().clone(), options)?;
    let shutdown = worker.shutdown_handle();
    let large_text = "x".repeat(128 * 1024);
    let image = vec![42_u8; 128 * 1024];
    let mut state = AgentRunState::new(
        "local-durable-run",
        vec![
            Message::user(large_text.clone()),
            Message::image_inline(image.clone(), "image/png"),
        ],
        AgentRunBudget {
            model_call_limit: 2,
        },
    );
    // This payload-specific test uses a scripted provider without an exact large context window.
    // Production providers retain normal context admission unless they explicitly opt out.
    state.plan.provider_enforced_limit_opt_out = true;
    let durable_runtime = TemporalRuntime::new(environment.client().clone());
    let history_client = environment.client().clone();
    let client_result = async move {
        let handle = durable_runtime
            .start(queue, state, TemporalRunConfig::new(1)?)
            .await?;
        let reconnected = durable_runtime.reconnect(handle.workflow_id());
        let result = reconnected.result().await?;
        let history = history_client
            .get_workflow_handle::<AgentiveWorkflow>(handle.workflow_id())
            .fetch_history(WorkflowFetchHistoryOptions::default())
            .to_json()
            .await?;
        Ok::<(AgentRunState, Vec<u8>), Box<dyn std::error::Error>>((result, history))
    };
    let worker_run = worker.run();
    tokio::pin!(worker_run);
    tokio::pin!(client_result);
    let (result, history) = tokio::select! {
        result = &mut client_result => result?,
        worker_result = &mut worker_run => return Err(format!("worker stopped before workflow result: {worker_result:?}").into()),
    };

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.text.as_deref(), Some("local durable result"));
    assert_eq!(result.history[0], Message::user(large_text));
    assert_eq!(result.history[1], Message::image_inline(image, "image/png"));
    let history = String::from_utf8(history)?;
    assert!(
        history
            .matches("YWdlbnRpdmUuaW8vZXh0ZXJuYWwtc3RvcmFnZS12MQ==")
            .count()
            >= 4,
        "every recorded workflow/activity payload must use the external-storage codec"
    );
    assert!(
        history.contains("\"maximumAttempts\":1"),
        "Temporal must not retry Agentive effects beyond their canonical retry policy"
    );
    scripted.assert_finished();
    shutdown();
    worker_run.await?;
    environment.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn committed_external_payload_history_replays_with_the_configured_converter()
-> Result<(), Box<dyn std::error::Error>> {
    let storage = FilesystemStorage::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/agentive_workflow_payloads"
    ))?;
    let converter = temporal_data_converter(PayloadPipeline::single_store(
        storage,
        StorageId::new("local-e2e")?,
    )?);
    let history = WorkflowHistory::from_json(&std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/agentive_workflow_history.json"),
    )?)?;
    let replayer = WorkflowReplayer::new(
        WorkflowReplayerOptions::new()
            .data_converter(converter)
            .register_workflow::<AgentiveWorkflow>()?
            .build(),
    )?;
    replayer.replay_workflow(history).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "downloads and starts the official Temporal CLI dev server; run before release"]
async fn cancelling_a_running_effect_prevents_a_late_provider_completion()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let pipeline = PayloadPipeline::single_store(
        FilesystemStorage::new(directory.path())?,
        StorageId::new("cancel-e2e")?,
    )?;
    let environment = WorkflowEnvironment::start_local(
        LocalWorkflowEnvironmentOptions::builder()
            .client_options(
                ClientOptions::new("default")
                    .data_converter(temporal_data_converter(pipeline))
                    .build(),
            )
            .build(),
    )
    .await?;
    let queue = format!("agentive-cancel-{}", ObjectId::random().to_hex());
    let scripted = ScriptedProvider::new()
        .delay(std::time::Duration::from_secs(30))
        .respond_with_text("late result");
    let options = agentive_temporal::runtime::worker_options(
        queue.clone(),
        AgentiveActivities::new(Agent::builder().provider(scripted.clone()).build()?),
    )?;
    let sdk_runtime = Runtime::from_current_tokio(Default::default())?;
    let mut worker = Worker::new(&sdk_runtime, environment.client().clone(), options)?;
    let shutdown = worker.shutdown_handle();
    let runtime = TemporalRuntime::new(environment.client().clone());
    let state = AgentRunState::new(
        "cancel-durable-run",
        vec![],
        AgentRunBudget {
            model_call_limit: 1,
        },
    );
    let handle = runtime
        .start(queue, state, TemporalRunConfig::default())
        .await?;
    let worker_run = worker.run();
    tokio::pin!(worker_run);
    for _ in 0..50 {
        if scripted.recorded_requests().len() == 1 {
            break;
        }
        tokio::select! {
            worker_result = &mut worker_run => return Err(format!("worker stopped before provider activity: {worker_result:?}").into()),
            () = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
        }
    }
    scripted.assert_request_count(1);
    handle.cancel().await?;
    let cancelled = tokio::select! {
        result = handle.result() => result.expect_err("cancelled workflow must not return a late result"),
        worker_result = &mut worker_run => return Err(format!("worker stopped before cancellation: {worker_result:?}").into()),
    };
    assert!(matches!(
        cancelled,
        WorkflowGetResultError::Cancelled { .. }
    ));
    assert_eq!(
        scripted.cancellation_count(),
        1,
        "the running provider must observe Agentive's canonical cancellation token"
    );
    shutdown();
    worker_run.await?;
    environment.shutdown().await?;
    Ok(())
}
