#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

use agentive::{
    CancellationReason, CancellationToken, ModelOutputFormat, ModelRequest, ModelResponse,
    ProviderError, ProviderErrorKind, Tool, ToolDefinition, ToolHandle, validate_tool_definition,
};
use futures::future::BoxFuture;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::watch;

mod errors;
mod protocol;
mod transport;
use errors::{
    cancelled_error, classify_server_error, classify_turn_failure, phase_timeout_error,
    protocol_error, transport_error,
};
use protocol::{notification, request_response};
pub use transport::StdioTransport;

const SUPPORTED_PROTOCOL_VERSION: &str = "2";
const PROTOCOL_PHASE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug)]
pub(crate) enum PreflightError {
    InvalidToolConfiguration,
    Unsupported(&'static str),
}

impl PreflightError {
    fn into_provider_error(self) -> ProviderError {
        let message = match self {
            Self::InvalidToolConfiguration => "codex tool configuration is invalid".to_owned(),
            Self::Unsupported(capability) => {
                format!("codex app server does not support lossless {capability} input")
            }
        };
        ProviderError::terminal(ProviderErrorKind::InvalidRequest, message)
    }
}

/// A factory for isolated App Server connections.
///
/// This is primarily a controlled test seam. Production callers should use
/// [`CodexRuntime::new`], which starts `codex app-server` on stdio.
pub trait CodexTransport: Clone + Send + Sync + 'static {
    /// Opens one isolated App Server session.
    fn open(&self) -> BoxFuture<'static, Result<Box<dyn CodexSession>, ProviderError>>;
}

/// A single bidirectional JSONL App Server connection.
///
/// The JSON-RPC wire shape remains an implementation detail of the runtime;
/// this trait exists solely to make protocol fixtures deterministic.
pub trait CodexSession: Send {
    /// Writes one protocol message.
    fn send(&mut self, message: Value) -> BoxFuture<'_, Result<(), ProviderError>>;

    /// Receives one protocol message, or `None` on clean end of stream.
    fn receive(&mut self) -> BoxFuture<'_, Result<Option<Value>, ProviderError>>;

    /// Releases the session and waits until its backing transport is reaped.
    ///
    /// Test transports may rely on this no-op default when they do not own an
    /// external process.
    fn shutdown(&mut self) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(async { Ok(()) })
    }
}

/// A subscription-backed runtime over the local Codex App Server.
///
/// App Server owns turn orchestration, so this type intentionally does not
/// implement Agentive's one-model-turn `ModelProvider` trait.
#[derive(Clone)]
pub struct CodexRuntime<T = StdioTransport> {
    transport: T,
    tools: Vec<ToolHandle>,
}

/// A tool and the one immutable declaration used for an App Server turn.
#[derive(Clone)]
pub(crate) struct FrozenTool {
    pub(crate) tool: ToolHandle,
    pub(crate) definition: ToolDefinition,
}

/// Explicit cancellation for one Codex runtime turn.
#[derive(Clone, Default)]
pub struct CodexCancellation {
    token: CancellationToken,
}

impl CodexCancellation {
    /// Requests cancellation. The active turn receives `turn/interrupt`.
    pub fn cancel(&self) {
        self.token.cancel(CancellationReason::User);
    }

    fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

impl CodexRuntime<StdioTransport> {
    /// Uses `codex app-server` with its existing local subscription session.
    #[must_use]
    pub fn new() -> Self {
        Self {
            transport: StdioTransport::default(),
            tools: Vec::new(),
        }
    }

    /// Uses an explicitly installed Codex executable without accepting tokens.
    #[must_use]
    pub fn with_executable(path: impl Into<PathBuf>) -> Self {
        Self {
            transport: StdioTransport {
                executable: path.into(),
            },
            tools: Vec::new(),
        }
    }
}

impl Default for CodexRuntime<StdioTransport> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> CodexRuntime<T>
where
    T: CodexTransport,
{
    /// Constructs the runtime with a controlled process/transport seam.
    #[must_use]
    pub fn with_transport(transport: T) -> Self {
        Self {
            transport,
            tools: Vec::new(),
        }
    }

    /// Adds one canonical Agentive tool for App Server's callback flow.
    #[must_use]
    pub fn tool<U: Tool + 'static>(mut self, tool: U) -> Self {
        self.tools.push(ToolHandle::new(tool));
        self
    }

    /// Adds pre-erased canonical tool handles for a heterogeneous registry.
    #[must_use]
    pub fn with_tools(mut self, tools: impl IntoIterator<Item = ToolHandle>) -> Self {
        self.tools = tools.into_iter().collect();
        self
    }

    /// Starts an isolated turn and returns a handle for explicit cancellation.
    #[must_use]
    pub fn start(&self, request: ModelRequest) -> CodexRunHandle {
        let cancellation = CodexCancellation::default();
        let runtime = self.clone();
        let cancellation_for_task = cancellation.clone();
        let (result, receiver) = watch::channel(None);
        tokio::spawn(async move {
            result.send_replace(Some(
                runtime
                    .run_cancellable(request, &cancellation_for_task)
                    .await,
            ));
        });
        CodexRunHandle {
            cancellation,
            receiver,
        }
    }

    /// Executes one isolated Codex App Server turn.
    ///
    /// # Errors
    ///
    /// Returns a classified canonical provider error when the request cannot
    /// map losslessly, App Server rejects the protocol, or the local process
    /// fails. Error messages intentionally omit server diagnostics.
    pub async fn run(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        self.run_cancellable(request, &CodexCancellation::default())
            .await
    }

    /// Executes one turn and sends `turn/interrupt` if `cancellation` fires.
    ///
    /// # Errors
    ///
    /// Returns only classified, secret-safe canonical provider errors.
    pub async fn run_cancellable(
        &self,
        request: ModelRequest,
        cancellation: &CodexCancellation,
    ) -> Result<ModelResponse, ProviderError> {
        if !matches!(&request.output_format, ModelOutputFormat::Text) {
            return Err(ProviderError::terminal(
                ProviderErrorKind::InvalidRequest,
                "codex app server structured output is not supported losslessly",
            ));
        }
        if !request.include_context {
            return Err(PreflightError::Unsupported("context exclusion").into_provider_error());
        }
        if request.max_output_tokens != 0 {
            return Err(
                PreflightError::Unsupported("exact output-token limits").into_provider_error()
            );
        }
        let tools =
            freeze_tools(&request, &self.tools).map_err(PreflightError::into_provider_error)?;
        let input = compile_input(&request).map_err(PreflightError::into_provider_error)?;
        let mut session = cancellable_phase(cancellation, self.transport.open()).await?;

        let turn_result = async {
            let initialized = cancellable_phase(cancellation, request_response(
                session.as_mut(),
                1,
                "initialize",
                json!({
                    "clientInfo": {"name": "agentive-codex", "version": env!("CARGO_PKG_VERSION")},
                    "capabilities": {"experimentalApi": true},
                }),
            ))
            .await?;
            if let Some(version) = initialized
                .get("protocolVersion")
                .or_else(|| initialized.pointer("/serverInfo/version"))
                .and_then(Value::as_str)
                && version != SUPPORTED_PROTOCOL_VERSION
            {
                return Err(protocol_error());
            }
            cancellable_phase(
                cancellation,
                notification(session.as_mut(), "initialized", json!({})),
            )
            .await?;

            let thread = cancellable_phase(cancellation, request_response(
                session.as_mut(),
                2,
                "thread/start",
                json!({"model": request.model, "dynamicTools": dynamic_tools(&tools), "developerInstructions": request.instructions.render_xml()}),
            ))
            .await?;
            let thread_id = string_at(&thread, "/thread/id").ok_or_else(protocol_error)?;

            let turn = cancellable_phase(cancellation, request_response(
                session.as_mut(),
                3,
                "turn/start",
                json!({
                    "threadId": thread_id,
                    "input": input,
                    "model": request.model,
                }),
            ))
            .await?;
            let turn_id = string_at(&turn, "/turn/id").ok_or_else(protocol_error)?;
            collect_turn(session.as_mut(), &thread_id, &turn_id, cancellation, &tools).await
        }
        .await;
        let shutdown_result = session.shutdown().await;
        match turn_result {
            Err(error) => Err(error),
            Ok(response) => {
                shutdown_result?;
                Ok(response)
            }
        }
    }
}

async fn cancellable_phase<F, O>(
    cancellation: &CodexCancellation,
    future: F,
) -> Result<O, ProviderError>
where
    F: Future<Output = Result<O, ProviderError>>,
{
    tokio::select! {
        biased;
        _ = cancellation.token.cancelled() => Err(cancelled_error()),
        result = tokio::time::timeout(PROTOCOL_PHASE_TIMEOUT, future) => {
            result.map_err(|_| phase_timeout_error())?
        }
    }
}

fn freeze_tools(
    request: &ModelRequest,
    tools: &[ToolHandle],
) -> Result<Vec<FrozenTool>, PreflightError> {
    let mut declared = HashMap::with_capacity(request.tools.len());
    for descriptor in &request.tools {
        validate_tool_definition(&descriptor.description, &descriptor.schema)
            .map_err(|_| PreflightError::InvalidToolConfiguration)?;
        if declared
            .insert(descriptor.name.as_str(), descriptor)
            .is_some()
        {
            return Err(PreflightError::InvalidToolConfiguration);
        }
    }

    let mut frozen = Vec::with_capacity(tools.len());
    let mut registered = HashSet::with_capacity(tools.len());
    for tool in tools {
        let definition = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tool.metadata()))
            .map_err(|_| PreflightError::InvalidToolConfiguration)?;
        validate_tool_definition(definition.description, &definition.schema.json)
            .map_err(|_| PreflightError::InvalidToolConfiguration)?;
        if !registered.insert(definition.name.as_str().to_owned()) {
            return Err(PreflightError::InvalidToolConfiguration);
        }
        if let Some(descriptor) = declared.get(definition.name.as_str())
            && (descriptor.description != definition.description
                || descriptor.schema != definition.schema.json
                || descriptor.idempotent != definition.idempotent)
        {
            return Err(PreflightError::InvalidToolConfiguration);
        }
        frozen.push(FrozenTool {
            tool: tool.clone(),
            definition,
        });
    }
    if declared.len() != registered.len() || declared.keys().any(|name| !registered.contains(*name))
    {
        return Err(PreflightError::InvalidToolConfiguration);
    }
    Ok(frozen)
}

/// A running isolated Codex turn.
pub struct CodexRunHandle {
    cancellation: CodexCancellation,
    receiver: watch::Receiver<Option<Result<ModelResponse, ProviderError>>>,
}

impl CodexRunHandle {
    /// Sends `turn/interrupt` to the active App Server turn.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Waits for the canonical response or classified terminal error.
    pub async fn wait(&self) -> Result<ModelResponse, ProviderError> {
        let mut receiver = self.receiver.clone();
        loop {
            if let Some(result) = receiver.borrow().clone() {
                return result;
            }
            receiver.changed().await.map_err(|_| transport_error())?;
        }
    }
}

mod turn;
use turn::{collect_turn, compile_input, dynamic_tools, string_at};
