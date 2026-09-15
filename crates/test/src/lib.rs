#![allow(missing_docs)] // Test-support helpers are documented in the public guide.
#![allow(clippy::expect_used)] // Assertion-style test helpers retain their conventional panic API.

use agentive::ModelCapabilities;
use agentive::{
    ModelFinishReason, ModelProvider, ModelRequest, ModelResponse, ModelStreamEvent,
    ModelTokenUsage, ModelToolCall, ProviderCallContext, ProviderError, ProviderErrorKind,
    StructuredOutputSupport, UsageEstimator,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

mod conformance;
pub use conformance::{
    ProviderConformanceError, ProviderConformanceHarness, ScriptedProviderHarness,
    assert_provider_conformance,
};

#[derive(Debug, Clone)]
pub enum ScriptedResult {
    ExpectedRequest(ModelRequest),
    Response(ModelResponse),
    Stream(Vec<Result<ModelStreamEvent, ProviderError>>),
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
    structured_output_support: Mutex<StructuredOutputSupport>,
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
                max_context_tokens: Some(8192),
            }),
            structured_output_support: Mutex::new(StructuredOutputSupport::JsonSchema),
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

    /// Requires the next provider operation to receive this exact canonical request.
    pub fn expect_request(self, request: ModelRequest) -> Self {
        self.push(ScriptedResult::ExpectedRequest(request))
    }

    /// Appends one native provider stream, including its required terminal event.
    pub fn respond_with_stream(self, events: Vec<Result<ModelStreamEvent, ProviderError>>) -> Self {
        if let Ok(mut capabilities) = self.inner.capabilities.lock() {
            capabilities.supports_streaming = true;
        }
        self.push(ScriptedResult::Stream(events))
    }

    /// Overrides the structured-output guarantee advertised by this provider.
    pub fn structured_output_support(self, support: StructuredOutputSupport) -> Self {
        if let Ok(mut stored) = self.inner.structured_output_support.lock() {
            *stored = support;
        }
        self
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
}

impl Clone for ScriptedProvider {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl ModelProvider for ScriptedProvider {
    fn structured_output_support(&self) -> StructuredOutputSupport {
        self.inner
            .structured_output_support
            .lock()
            .map_or(StructuredOutputSupport::None, |support| *support)
    }
    fn conservative_context_token_bound(
        &self,
        request: &ModelRequest,
    ) -> Result<Option<u64>, String> {
        UsageEstimator
            .conservative_context_token_bound(request)
            .map(Some)
    }

    fn exact_context_token_count(&self, request: &ModelRequest) -> Result<Option<u64>, String> {
        serde_json::to_vec(request)
            .map(|rendered| Some(u64::try_from(rendered.len()).unwrap_or(u64::MAX)))
            .map_err(|error| format!("could not render scripted model request: {error}"))
    }

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
                .push(request.clone());

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
                    ScriptedResult::ExpectedRequest(expected) => {
                        if request != expected {
                            return Err(ProviderError::terminal(
                                ProviderErrorKind::Protocol,
                                "scripted provider request did not match expectation",
                            ));
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
                                    "scripted provider exhausted after expectation",
                                )
                            })?;
                    }
                    ScriptedResult::Response(response) => return Ok(response),
                    ScriptedResult::Error(error) => return Err(error),
                    ScriptedResult::Stream(_) => {
                        return Err(ProviderError::terminal(
                            ProviderErrorKind::Protocol,
                            "scripted stream was consumed through generate",
                        ));
                    }
                    ScriptedResult::Delay(duration) => {
                        let notified = context.cancel_notifier.notified();
                        if context.is_cancelled() {
                            this.inner.cancellations.fetch_add(1, Ordering::SeqCst);
                            return Err(ProviderError::terminal(
                                ProviderErrorKind::Unknown,
                                "scripted provider observed cancellation",
                            ));
                        }
                        tokio::select! {
                            () = sleep(duration) => {}
                            () = notified => {
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

    fn stream<'call>(
        &'call self,
        request: ModelRequest,
        context: &'call ProviderCallContext,
    ) -> impl std::future::Future<Output = Result<agentive::ModelStream, ProviderError>> + Send + 'call
    {
        let this = self.clone();
        Box::pin(async move {
            this.inner
                .requests
                .lock()
                .expect("request lock")
                .push(request.clone());
            loop {
                let next = this
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
                            "scripted provider exhausted",
                        )
                    })?;
                match next {
                    ScriptedResult::ExpectedRequest(expected) => {
                        if request != expected {
                            return Err(ProviderError::terminal(
                                ProviderErrorKind::Protocol,
                                "scripted provider request did not match expectation",
                            ));
                        }
                    }
                    ScriptedResult::Stream(events) => {
                        return Ok(Box::pin(futures::stream::iter(events)) as agentive::ModelStream);
                    }
                    ScriptedResult::Error(error) => return Err(error),
                    ScriptedResult::Response(_) => {
                        return Err(ProviderError::terminal(
                            ProviderErrorKind::Protocol,
                            "scripted response was consumed through stream",
                        ));
                    }
                    ScriptedResult::Delay(duration) => {
                        let notified = context.cancel_notifier.notified();
                        if context.is_cancelled() {
                            this.inner.cancellations.fetch_add(1, Ordering::SeqCst);
                            return Err(ProviderError::terminal(
                                ProviderErrorKind::Unknown,
                                "scripted provider observed cancellation",
                            ));
                        }
                        tokio::select! {
                            () = sleep(duration) => {}
                            () = notified => {
                                this.inner.cancellations.fetch_add(1, Ordering::SeqCst);
                                return Err(ProviderError::terminal(
                                    ProviderErrorKind::Unknown,
                                    "scripted provider observed cancellation",
                                ));
                            }
                        }
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
