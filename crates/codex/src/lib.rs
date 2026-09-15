#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

use agentive::{
    CancellationReason, CancellationToken, ModelRequest, ModelResponse, ProviderError, Tool,
};
use futures::future::BoxFuture;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::watch;

mod errors;
mod protocol;
mod transport;
use errors::{classify_server_error, classify_turn_failure, protocol_error, transport_error};
use protocol::{notification, request_response};
pub use transport::StdioTransport;

const SUPPORTED_PROTOCOL_VERSION: &str = "2";

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
}

/// A subscription-backed runtime over the local Codex App Server.
///
/// App Server owns turn orchestration, so this type intentionally does not
/// implement Agentive's one-model-turn `ModelProvider` trait.
#[derive(Clone)]
pub struct CodexRuntime<T = StdioTransport> {
    transport: T,
    tools: Vec<Arc<dyn Tool>>,
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

    /// Adds canonical Agentive tools for App Server's experimental callback flow.
    #[must_use]
    pub fn with_tools(mut self, tools: impl IntoIterator<Item = Arc<dyn Tool>>) -> Self {
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
            let _ = result.send(Some(
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
        let input = compile_input(&request)?;
        let mut session = self.transport.open().await?;

        let initialized = request_response(
            session.as_mut(),
            1,
            "initialize",
            json!({
                "clientInfo": {"name": "agentive-codex", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {"experimentalApi": true},
            }),
        )
        .await?;
        if let Some(version) = initialized
            .get("protocolVersion")
            .or_else(|| initialized.pointer("/serverInfo/version"))
            .and_then(Value::as_str)
            && version != SUPPORTED_PROTOCOL_VERSION
        {
            return Err(protocol_error());
        }
        notification(session.as_mut(), "initialized", json!({})).await?;

        let thread = request_response(
            session.as_mut(),
            2,
            "thread/start",
            json!({"model": request.model, "dynamicTools": dynamic_tools(&request, &self.tools), "developerInstructions": request.instructions.render_xml()}),
        )
        .await?;
        let thread_id = string_at(&thread, "/thread/id").ok_or_else(protocol_error)?;

        let turn = request_response(
            session.as_mut(),
            3,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": input,
                "model": request.model,
            }),
        )
        .await?;
        let turn_id = string_at(&turn, "/turn/id").ok_or_else(protocol_error)?;
        collect_turn(
            session.as_mut(),
            &thread_id,
            &turn_id,
            cancellation,
            &self.tools,
        )
        .await
    }
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
