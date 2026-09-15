//! Temporal SDK activity-environment coverage for the canonical durable effect seam.

#![allow(clippy::expect_used, clippy::manual_async_fn)] // Explicit RPITIT fixtures.

use agentive::{
    Agent, AgentRunBudget, AgentRunState, Message, ModelCapabilities, ModelFinishReason,
    ModelProvider, ModelRequest, ModelResponse, ModelToolCall, ProviderCallContext, ProviderError,
    ProviderErrorKind, RunStatus, Tool, ToolContext, ToolName,
};
use agentive_temporal::runtime::{
    AgentiveActivities, AgentiveWorkflow, EffectActivityInput, TemporalRunConfig, TemporalRuntime,
};
use agentive_temporal::{
    FilesystemStorage, ObjectId, PayloadLimits, PayloadPipeline, StorageId, temporal_data_converter,
};
use agentive_test::ScriptedProvider;
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use temporalio_client::{
    Client, ClientOptions, ConnectionOptions, WorkflowFetchHistoryOptions, WorkflowHistory,
};
use temporalio_sdk::testing::{
    ActivityEnvironment, LocalWorkflowEnvironmentOptions, WorkflowEnvironment,
};
use temporalio_sdk::{
    Runtime, Worker,
    workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions},
};

#[derive(Clone)]
struct ImageCapableScriptedProvider(ScriptedProvider);

struct NoopTool {
    name: ToolName,
    calls: Arc<AtomicUsize>,
}

impl NoopTool {
    fn new() -> Self {
        Self {
            name: "noop".parse().expect("valid fixture tool name"),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Tool for NoopTool {
    fn name(&self) -> &ToolName {
        &self.name
    }
    fn description(&self) -> &'static str {
        "completes exactly once"
    }
    fn schema_json(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| json!({"type":"object","additionalProperties":false}))
    }
    fn call<'a>(
        &'a self,
        _: &'a ToolContext,
        _: Value,
    ) -> impl std::future::Future<Output = Result<Value, agentive::ToolError>> + Send + 'a {
        let calls = Arc::clone(&self.calls);
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"ok": true}))
        }
    }
}

impl ModelProvider for ImageCapableScriptedProvider {
    fn conservative_context_token_bound(
        &self,
        request: &ModelRequest,
    ) -> Result<Option<u64>, String> {
        self.0.conservative_context_token_bound(request)
    }

    fn exact_context_token_count(&self, request: &ModelRequest) -> Result<Option<u64>, String> {
        self.0.exact_context_token_count(request)
    }

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
            max_context_tokens: Some(1_000_000),
        }
    }
}

#[tokio::test]
async fn activity_environment_executes_and_commits_one_canonical_provider_effect()
-> Result<(), Box<dyn std::error::Error>> {
    let provider = ScriptedProvider::new().respond_with_text("durably done");
    let agent = Agent::builder().provider(provider.clone()).build()?;
    let activities = AgentiveActivities::new(agent.clone());
    let mut state = agent.prepare_state(
        "durable-activity",
        vec![],
        &agentive::RunOptions {
            model_call_limit: 1,
            ..agentive::RunOptions::default()
        },
    )?;
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

    let (outcome, elapsed_ms) = outcome.into_parts();
    state.commit_effect(&effect_id, outcome)?;
    state.record_elapsed(elapsed_ms);
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
    let agent = Agent::builder()
        .provider(provider.clone())
        .tool(NoopTool::new())
        .build()?;
    let options = agentive_temporal::runtime::worker_options(
        queue.clone(),
        AgentiveActivities::new(agent.clone()),
    )?;
    let runtime = Runtime::from_current_tokio(Default::default())?;
    let mut worker = Worker::new(&runtime, environment.client().clone(), options)?;
    let shutdown = worker.shutdown_handle();
    let large_text = "x".repeat(128 * 1024);
    let image = vec![42_u8; 128 * 1024];
    let state = agent.prepare_state(
        "local-durable-run",
        vec![
            Message::user(large_text.clone()),
            Message::image_inline(image.clone(), "image/png").expect("valid image"),
        ],
        &agentive::RunOptions {
            model_call_limit: 2,
            // This payload-specific test uses a scripted provider without an exact large context
            // window. Production providers retain normal admission unless explicitly opted out.
            provider_enforced_limit_opt_out: true,
            ..agentive::RunOptions::default()
        },
    )?;
    let durable_runtime = TemporalRuntime::new(environment.client().clone());
    let history_client = environment.client().clone();
    let client_result = async move {
        let handle = durable_runtime
            .start(
                queue,
                state,
                TemporalRunConfig::new(1)?.allow_durable_idempotent_tool("noop".parse()?),
            )
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
    assert_eq!(
        result.history[1],
        Message::image_inline(image, "image/png").expect("valid image")
    );
    let history = String::from_utf8(history)?;
    assert!(
        history
            .matches("YWdlbnRpdmUuaW8vZXh0ZXJuYWwtc3RvcmFnZS12MQ==")
            .count()
            >= 4,
        "every recorded workflow/activity payload must use the external-storage codec"
    );
    scripted.assert_finished();
    shutdown();
    worker_run.await?;
    environment.shutdown().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "downloads and starts the official Temporal CLI dev server; run before release"]
async fn local_temporal_worker_restart_recovers_an_active_provider_effect_once()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let port = available_port()?;
    let _server = DevServer::start(port)?;
    wait_for_server(port).await?;
    let client = crash_client(port, directory.path()).await?;
    let queue = format!("agentive-restart-{}", ObjectId::random().to_hex());
    let durable_runtime = TemporalRuntime::new(client.clone());
    let state = persistent_agent(directory.path(), "recover")?.prepare_state(
        "restart-durable-run",
        vec![Message::user("x".repeat(128 * 1024))],
        &agentive::RunOptions {
            model_call_limit: 2,
            provider_enforced_limit_opt_out: true,
            ..agentive::RunOptions::default()
        },
    )?;
    let handle = durable_runtime
        .start(
            queue.clone(),
            state,
            TemporalRunConfig::default()
                .allow_durable_idempotent_provider()
                .allow_durable_idempotent_tool("noop".parse()?),
        )
        .await?;
    let mut first = spawn_crash_worker(
        "block",
        port,
        directory.path(),
        &queue,
        handle.workflow_id(),
    )?;
    wait_for_file(&directory.path().join("provider-started")).await?;
    first.kill()?;
    assert!(
        !first.wait()?.success(),
        "the first worker must be killed mid-provider effect"
    );

    let mut second = spawn_crash_worker(
        "recover",
        port,
        directory.path(),
        &queue,
        handle.workflow_id(),
    )?;
    let result = tokio::time::timeout(Duration::from_secs(45), handle.result()).await??;
    assert!(
        second.wait()?.success(),
        "the replacement worker must complete cleanly"
    );

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.text.as_deref(), Some("recovered exactly once"));
    let tool_invocation_file = directory.path().join("tool-invocations");
    assert!(
        tool_invocation_file.exists(),
        "tool was never invoked; fixture root is {}",
        directory.path().display()
    );
    assert_eq!(
        lines(&tool_invocation_file)?.len(),
        1,
        "the tool effect is committed once; Temporal transport retries require this stable idempotency key"
    );
    assert_eq!(
        result
            .history
            .iter()
            .filter(|message| message.role == agentive::MessageRole::Assistant)
            .count(),
        2,
        "only the recovered tool-call turn and final turn are committed",
    );
    let provider_invocations = lines(&directory.path().join("provider-invocations"))?;
    assert_eq!(provider_invocations.len(), 3);
    assert_eq!(
        provider_invocations[0], provider_invocations[1],
        "the redelivered provider effect keeps its stable identity"
    );
    assert_eq!(provider_invocations[0], "restart-durable-run:provider:0");
    assert_eq!(
        lines(&tool_invocation_file)?[0],
        "restart-durable-run:tool:durable-call|restart-durable-run:durable-call"
    );
    let history = client
        .get_workflow_handle::<AgentiveWorkflow>(handle.workflow_id())
        .fetch_history(WorkflowFetchHistoryOptions::default())
        .to_json()
        .await?;
    assert!(
        String::from_utf8(history)?
            .matches("YWdlbnRpdmUuaW8vZXh0ZXJuYWwtc3RvcmFnZS12MQ==")
            .count()
            >= 4,
        "workflow and activity payloads must remain always external"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "starts the official Temporal CLI dev server; run before release"]
async fn local_temporal_agentive_retry_is_not_multiplied_by_temporal_retries()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let port = available_port()?;
    let _server = DevServer::start(port)?;
    wait_for_server(port).await?;
    let client = crash_client(port, directory.path()).await?;
    let queue = format!("agentive-retry-{}", ObjectId::random().to_hex());
    let state = persistent_agent(directory.path(), "retry-crash")?.prepare_state(
        "retry-durable-run",
        vec![Message::user("retry once")],
        &agentive::RunOptions {
            model_call_limit: 2,
            provider_max_attempts: 2,
            ..agentive::RunOptions::default()
        },
    )?;
    let handle = TemporalRuntime::new(client.clone())
        .start(
            queue.clone(),
            state,
            TemporalRunConfig::default().allow_durable_idempotent_provider(),
        )
        .await?;
    let mut first = spawn_crash_worker(
        "retry-crash",
        port,
        directory.path(),
        &queue,
        handle.workflow_id(),
    )?;
    wait_for_file(&directory.path().join("provider-upstream-complete")).await?;
    first.kill()?;
    assert!(
        !first.wait()?.success(),
        "the first worker must be killed after Agentive's retry succeeds but before completion"
    );
    let mut second = spawn_crash_worker(
        "retry-crash",
        port,
        directory.path(),
        &queue,
        handle.workflow_id(),
    )?;
    let result = tokio::time::timeout(Duration::from_secs(45), handle.result()).await??;
    assert!(second.wait()?.success());
    assert_eq!(result.text.as_deref(), Some("agentive retry completed"));
    let deliveries = lines(&directory.path().join("provider-invocations"))?;
    assert_eq!(
        deliveries,
        [
            "retry-durable-run:provider:0",
            "retry-durable-run:provider:0",
            "retry-durable-run:provider:0",
        ]
    );
    assert_eq!(
        lines(&directory.path().join("provider-upstream-attempts"))?.len(),
        2,
        "the redelivered activity must use the provider's durable effect cache instead of starting a second Agentive retry sequence"
    );
    Ok(())
}

#[tokio::test]
async fn crash_worker_process() -> Result<(), Box<dyn std::error::Error>> {
    let Ok(mode) = std::env::var("AGENTIVE_CRASH_MODE") else {
        return Ok(());
    };
    let port: u16 = std::env::var("AGENTIVE_CRASH_PORT")?.parse()?;
    let root = PathBuf::from(std::env::var("AGENTIVE_CRASH_ROOT")?);
    let queue = std::env::var("AGENTIVE_CRASH_QUEUE")?;
    let workflow_id = std::env::var("AGENTIVE_CRASH_WORKFLOW_ID")?;
    let client = crash_client(port, &root).await?;
    let agent = persistent_agent(&root, &mode)?;
    let options =
        agentive_temporal::runtime::worker_options(queue, AgentiveActivities::new(agent))?;
    let runtime = Runtime::from_current_tokio(Default::default())?;
    let mut worker = Worker::new(&runtime, client.clone(), options)?;
    let shutdown = worker.shutdown_handle();
    let worker_run = worker.run();
    tokio::pin!(worker_run);
    let handle = TemporalRuntime::new(client).reconnect(workflow_id);
    tokio::select! {
        result = handle.result() => { result?; }
        result = &mut worker_run => return Err(format!("worker ended before workflow result: {result:?}").into()),
    }
    shutdown();
    worker_run.await?;
    Ok(())
}

#[tokio::test]
async fn committed_external_payload_history_replays_with_the_configured_converter()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture_directory = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/agentive_workflow_payloads"
    ));
    let replay_directory = tempfile::tempdir()?;
    copy_directory(fixture_directory, replay_directory.path())?;
    let storage = FilesystemStorage::new(replay_directory.path())?;
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

fn copy_directory(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let destination = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory(&entry.path(), &destination)?;
        } else {
            std::fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}

struct DevServer(Child);

impl DevServer {
    fn start(port: u16) -> std::io::Result<Self> {
        Command::new("temporal")
            .args([
                "server",
                "start-dev",
                "--headless",
                "--ip",
                "127.0.0.1",
                "--port",
                &port.to_string(),
            ])
            .spawn()
            .map(Self)
    }
}

impl Drop for DevServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct PersistentProvider {
    root: PathBuf,
    mode: String,
}

impl PersistentProvider {
    fn new(root: PathBuf, mode: String) -> Self {
        Self { root, mode }
    }

    fn record(&self, file: &str, line: &str) -> Result<usize, ProviderError> {
        append_line(&self.root.join(file), line).map_err(|error| {
            ProviderError::terminal(
                ProviderErrorKind::Unknown,
                format!("crash fixture could not persist {file}: {error}"),
            )
        })?;
        lines(&self.root.join(file))
            .map(|entries| entries.len())
            .map_err(|error| {
                ProviderError::terminal(
                    ProviderErrorKind::Unknown,
                    format!("crash fixture could not read {file}: {error}"),
                )
            })
    }
}

impl ModelProvider for PersistentProvider {
    fn conservative_context_token_bound(
        &self,
        request: &ModelRequest,
    ) -> Result<Option<u64>, String> {
        Ok(Some(
            u64::try_from(
                serde_json::to_vec(request)
                    .map_err(|error| error.to_string())?
                    .len(),
            )
            .unwrap_or(u64::MAX),
        ))
    }

    fn exact_context_token_count(&self, request: &ModelRequest) -> Result<Option<u64>, String> {
        self.conservative_context_token_bound(request)
    }

    async fn generate(
        &self,
        request: ModelRequest,
        _context: &ProviderCallContext,
    ) -> Result<ModelResponse, ProviderError> {
        let count = self.record("provider-invocations", &request.invocation_id)?;
        match self.mode.as_str() {
            "block" => {
                fs::write(self.root.join("provider-started"), b"started").map_err(|error| {
                    ProviderError::terminal(
                        ProviderErrorKind::Unknown,
                        format!("could not persist start marker: {error}"),
                    )
                })?;
                tokio::time::sleep(Duration::from_secs(300)).await;
                Err(ProviderError::terminal(
                    ProviderErrorKind::Unknown,
                    "blocked provider unexpectedly resumed",
                ))
            }
            "recover" => match count {
                2 => Ok(ModelResponse {
                    text: None,
                    tool_calls: vec![ModelToolCall {
                        call_id: "durable-call".into(),
                        name: "noop".parse::<ToolName>().map_err(|error| {
                            ProviderError::terminal(ProviderErrorKind::Protocol, error.to_string())
                        })?,
                        arguments: json!({}),
                        provider_call_id: Some(request.invocation_id),
                    }],
                    usage: None,
                    finish_reason: ModelFinishReason::ToolCalls,
                }),
                3 => Ok(ModelResponse {
                    text: Some("recovered exactly once".into()),
                    tool_calls: Vec::new(),
                    usage: None,
                    finish_reason: ModelFinishReason::Stop,
                }),
                _ => Err(ProviderError::terminal(
                    ProviderErrorKind::Protocol,
                    format!("unexpected recovered provider attempt {count}"),
                )),
            },
            "retry-crash" => self.retry_crash_response(&request.invocation_id),
            _ => Err(ProviderError::terminal(
                ProviderErrorKind::Protocol,
                format!("unexpected fixture mode {} attempt {count}", self.mode),
            )),
        }
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            supports_tool_calls: true,
            supports_streaming: false,
            supports_images: false,
            max_context_tokens: Some(1_000_000),
        }
    }
}

impl PersistentProvider {
    fn retry_crash_response(&self, invocation_id: &str) -> Result<ModelResponse, ProviderError> {
        let completed = self.root.join("provider-upstream-complete");
        if completed.exists() {
            return Ok(retry_completed_response());
        }
        let attempt = self.record("provider-upstream-attempts", invocation_id)?;
        if attempt == 1 {
            return Err(ProviderError::retryable(
                ProviderErrorKind::Transport,
                "agentive-owned retry",
            ));
        }
        if attempt == 2 {
            fs::write(completed, b"completed").map_err(|error| {
                ProviderError::terminal(
                    ProviderErrorKind::Unknown,
                    format!("could not persist provider completion: {error}"),
                )
            })?;
            std::thread::sleep(Duration::from_secs(300));
        }
        Err(ProviderError::terminal(
            ProviderErrorKind::Protocol,
            format!("unexpected durable provider attempt {attempt}"),
        ))
    }
}

fn retry_completed_response() -> ModelResponse {
    ModelResponse {
        text: Some("agentive retry completed".into()),
        tool_calls: Vec::new(),
        usage: None,
        finish_reason: ModelFinishReason::Stop,
    }
}

struct PersistentTool {
    root: PathBuf,
    name: ToolName,
}

impl PersistentTool {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            name: "noop".parse().expect("valid fixture tool name"),
        }
    }
}

impl Tool for PersistentTool {
    fn name(&self) -> &ToolName {
        &self.name
    }
    fn description(&self) -> &'static str {
        "records one durable idempotent effect"
    }
    fn schema_json(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| json!({"type":"object","additionalProperties":false}))
    }
    fn call<'a>(
        &'a self,
        context: &'a ToolContext,
        _: Value,
    ) -> impl std::future::Future<Output = Result<Value, agentive::ToolError>> + Send + 'a {
        async move {
            let invocation = context.invocation();
            append_line(
                &self.root.join("tool-invocations"),
                &format!(
                    "{}|{}",
                    invocation.invocation_id, invocation.idempotency_key
                ),
            )
            .map_err(|error| agentive::ToolError::terminal("fixture_persist", error.to_string()))?;
            Ok(json!({"ok": true}))
        }
    }
}

fn available_port() -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.local_addr().map(|address| address.port())
}

async fn wait_for_server(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    Ok(())
}

async fn wait_for_file(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await?;
    Ok(())
}

async fn crash_client(port: u16, root: &Path) -> Result<Client, Box<dyn std::error::Error>> {
    let storage = FilesystemStorage::new(root)?;
    let client_options = ClientOptions::new("default")
        .data_converter(temporal_data_converter(PayloadPipeline::single_store(
            storage,
            StorageId::new("crash-e2e")?,
        )?))
        .build();
    Ok(Client::connect(
        ConnectionOptions::new(url::Url::parse(&format!("http://127.0.0.1:{port}"))?).build(),
        client_options,
    )
    .await?)
}

fn spawn_crash_worker(
    mode: &str,
    port: u16,
    root: &Path,
    queue: &str,
    workflow_id: &str,
) -> std::io::Result<Child> {
    Command::new(std::env::current_exe()?)
        .args(["--exact", "crash_worker_process"])
        .env("AGENTIVE_CRASH_MODE", mode)
        .env("AGENTIVE_CRASH_PORT", port.to_string())
        .env("AGENTIVE_CRASH_ROOT", root)
        .env("AGENTIVE_CRASH_QUEUE", queue)
        .env("AGENTIVE_CRASH_WORKFLOW_ID", workflow_id)
        .spawn()
}

fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")
}

fn lines(path: &Path) -> std::io::Result<Vec<String>> {
    Ok(fs::read_to_string(path)?
        .lines()
        .map(ToOwned::to_owned)
        .collect())
}

fn persistent_agent(root: &Path, mode: &str) -> Result<Agent, agentive::RunError> {
    Agent::builder()
        .provider(PersistentProvider::new(root.to_path_buf(), mode.to_owned()))
        .tool(PersistentTool::new(root.to_path_buf()))
        .build()
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
        result = handle.result() => result?,
        worker_result = &mut worker_run => return Err(format!("worker stopped before cancellation: {worker_result:?}").into()),
    };
    assert_eq!(cancelled.status, RunStatus::Cancelled);
    assert!(cancelled.text.is_none());
    let snapshot = handle.observe().await?.expect("cancelled state snapshot");
    assert_eq!(snapshot.state.status, RunStatus::Cancelled);
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
