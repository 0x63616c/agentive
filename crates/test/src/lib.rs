#![allow(missing_docs)] // Test-support helpers are documented in the public guide.
#![allow(clippy::expect_used)] // Assertion-style test helpers retain their conventional panic API.

use agentive::ModelCapabilities;
use agentive::{
    ModelFinishReason, ModelProvider, ModelRequest, ModelResponse, ModelTokenUsage, ModelToolCall,
    ProviderCallContext, ProviderError, ProviderErrorKind,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

/// A provider response that violates the reusable conformance contract.
#[derive(Debug, thiserror::Error)]
pub enum ProviderConformanceError {
    /// The provider rejected the canonical request.
    #[error("provider rejected conformance request: {0}")]
    Provider(ProviderError),
    /// A response contained a malformed correlated call.
    #[error("provider returned malformed tool call: {0}")]
    MalformedToolCall(String),
    /// The provider advertised a capability it did not honor.
    #[error("provider capability mismatch: {0}")]
    Capability(String),
}

#[derive(Debug, Clone)]
pub enum ScriptedResult {
    Response(ModelResponse),
    Error(ProviderError),
    Delay(Duration),
}

#[derive(Debug)]
struct ScriptedInner {
    script: Mutex<VecDeque<ScriptedResult>>,
    requests: Mutex<Vec<ModelRequest>>,
    cancellations: AtomicUsize,
    next_tool_call_id: AtomicUsize,
    capabilities: Mutex<ModelCapabilities>,
}

impl Default for ScriptedInner {
    fn default() -> Self {
        Self {
            script: Mutex::new(VecDeque::new()),
            requests: Mutex::new(Vec::new()),
            cancellations: AtomicUsize::new(0),
            next_tool_call_id: AtomicUsize::new(0),
            capabilities: Mutex::new(ModelCapabilities {
                supports_tool_calls: true,
                supports_streaming: false,
                supports_images: false,
                supports_context_count_estimate: true,
                max_context_tokens: Some(8192),
                exact_context_counting: true,
            }),
        }
    }
}

#[derive(Debug, Default)]
pub struct ScriptedProvider {
    inner: Arc<ScriptedInner>,
}

impl ScriptedProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn respond_with_text(self, text: impl Into<String>) -> Self {
        self.push(ScriptedResult::Response(ModelResponse {
            text: Some(text.into()),
            tool_calls: Vec::new(),
            usage: Some(ModelTokenUsage::default()),
            finish_reason: ModelFinishReason::Stop,
        }))
    }

    /// Overrides the capabilities advertised by this deterministic provider.
    pub fn capabilities(self, capabilities: ModelCapabilities) -> Self {
        if let Ok(mut stored) = self.inner.capabilities.lock() {
            *stored = capabilities;
        }
        self
    }

    /// Appends an exact model response to the deterministic script.
    pub fn respond_with(self, response: ModelResponse) -> Self {
        self.push(ScriptedResult::Response(response))
    }

    pub fn respond_with_tool_calls(
        self,
        calls: Vec<(impl Into<String>, serde_json::Value)>,
    ) -> Self {
        self.push(ScriptedResult::Response(ModelResponse {
            text: None,
            tool_calls: calls
                .into_iter()
                .map(|(name, arguments)| ModelToolCall {
                    call_id: format!(
                        "scripted-tool-call-{}",
                        self.inner.next_tool_call_id.fetch_add(1, Ordering::Relaxed)
                    ),
                    name: name.into().parse().expect("tool name must be valid"),
                    arguments,
                    provider_call_id: None,
                })
                .collect(),
            usage: Some(ModelTokenUsage::default()),
            finish_reason: ModelFinishReason::ToolCalls,
        }))
    }

    pub fn respond_with_error(self, error: ProviderError) -> Self {
        self.push(ScriptedResult::Error(error))
    }

    pub fn delay(self, duration: Duration) -> Self {
        self.push(ScriptedResult::Delay(duration))
    }

    fn push(&self, result: ScriptedResult) -> Self {
        self.inner
            .script
            .lock()
            .expect("script lock")
            .push_back(result);
        self.clone()
    }

    pub fn assert_finished(&self) {
        assert!(
            self.inner.script.lock().expect("script lock").is_empty(),
            "scripted provider had unconsumed interactions",
        );
    }

    pub fn recorded_requests(&self) -> Vec<ModelRequest> {
        self.inner.requests.lock().expect("request lock").clone()
    }

    /// Returns how many scripted delays observed provider-call cancellation.
    pub fn cancellation_count(&self) -> usize {
        self.inner.cancellations.load(Ordering::SeqCst)
    }

    pub fn assert_request_count(&self, expected: usize) {
        assert_eq!(
            self.inner.requests.lock().expect("request lock").len(),
            expected
        );
    }

    pub fn assert_last_request_has_tools(&self, minimum: usize) {
        let requests = self.inner.requests.lock().expect("request lock");
        let last = requests
            .last()
            .unwrap_or_else(|| panic!("expected at least one request"));
        assert!(last.tools.len() >= minimum);
    }

    pub async fn conformance_test_step(&self, request: &ModelRequest) {
        assert!(
            !request.invocation_id.is_empty(),
            "invocation_id must be present"
        );
    }
}

impl Clone for ScriptedProvider {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl ModelProvider for ScriptedProvider {
    fn generate<'call>(
        &'call self,
        request: ModelRequest,
        context: &'call ProviderCallContext,
    ) -> impl std::future::Future<Output = Result<ModelResponse, ProviderError>> + Send + 'call
    {
        let this = self.clone();

        Box::pin(async move {
            this.inner
                .requests
                .lock()
                .expect("request lock")
                .push(request);

            let next = {
                let mut script = this.inner.script.lock().expect("script lock");
                script.pop_front()
            };

            let next = match next {
                Some(item) => item,
                None => {
                    return Err(ProviderError::terminal(
                        ProviderErrorKind::Unavailable,
                        "scripted provider exhausted",
                    ));
                }
            };

            let mut next = next;
            loop {
                match next {
                    ScriptedResult::Response(response) => return Ok(response),
                    ScriptedResult::Error(error) => return Err(error),
                    ScriptedResult::Delay(duration) => {
                        tokio::select! {
                            () = sleep(duration) => {}
                            () = context.cancel_notifier.notified() => {
                                this.inner.cancellations.fetch_add(1, Ordering::SeqCst);
                                return Err(ProviderError::terminal(
                                    ProviderErrorKind::Unknown,
                                    "scripted provider observed cancellation",
                                ));
                            }
                        }
                        next = this
                            .inner
                            .script
                            .lock()
                            .map_err(|_| {
                                ProviderError::terminal(
                                    ProviderErrorKind::Unavailable,
                                    "scripted provider lock poisoned",
                                )
                            })?
                            .pop_front()
                            .ok_or_else(|| {
                                ProviderError::terminal(
                                    ProviderErrorKind::Unavailable,
                                    "scripted provider exhausted after delay",
                                )
                            })?;
                    }
                }
            }
        })
    }

    fn capabilities(&self) -> ModelCapabilities {
        self.inner
            .capabilities
            .lock()
            .map_or_else(|_| ModelCapabilities::default(), |value| value.clone())
    }
}

pub async fn assert_provider_conformance<P>(provider: &P) -> Result<(), ProviderConformanceError>
where
    P: ModelProvider,
{
    let capabilities = provider.capabilities();
    let context = ProviderCallContext {
        provider_call_id: "conformance-call-id".to_string(),
        model_round: 0,
        attempt: 1,
        remaining_time: Some(Duration::from_secs(1)),
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        cancel_notifier: Arc::new(tokio::sync::Notify::new()),
    };

    let request = ModelRequest {
        instructions: agentive::CompiledInstructions {
            core: agentive::InstructionFragment::new("test", 1, "test"),
            agent: None,
            run: None,
            features: vec![],
        },
        messages: vec![agentive::Message::user("conformance input")],
        tools: vec![agentive::ProviderToolDescriptor {
            name: "conformance_tool"
                .parse::<agentive::ToolName>()
                .map_err(|error| ProviderConformanceError::MalformedToolCall(error.to_string()))?,
            description: "Verify canonical tool mapping.".to_string(),
            schema: serde_json::json!({"type":"object","additionalProperties":false}),
            idempotent: true,
        }],
        include_context: true,
        model: None,
        max_output_tokens: 16,
        invocation_id: "conformance-request".to_string(),
    };

    let response = provider
        .generate(request, &context)
        .await
        .map_err(ProviderConformanceError::Provider)?;
    if !capabilities.supports_tool_calls && !response.tool_calls.is_empty() {
        return Err(ProviderConformanceError::Capability(
            "provider returned tool calls while advertising no tool-call support".to_string(),
        ));
    }
    for call in response.tool_calls {
        if call.call_id.trim().is_empty() {
            return Err(ProviderConformanceError::MalformedToolCall(
                "tool call id is empty".to_string(),
            ));
        }
        if call.name.as_str().is_empty() {
            return Err(ProviderConformanceError::MalformedToolCall(
                "tool name is empty".to_string(),
            ));
        }
    }
    Ok(())
}
