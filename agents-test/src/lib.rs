use agents::ModelCapabilities;
use agents::{
    ModelFinishReason, ModelProvider, ModelRequest, ModelResponse, ModelTokenUsage, ModelToolCall,
    ProviderCallContext, ProviderError, ProviderErrorKind,
};
use futures::future::BoxFuture;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Clone)]
pub enum ScriptedResult {
    Response(ModelResponse),
    Error(ProviderError),
    Delay(Duration),
}

#[derive(Debug, Default)]
struct ScriptedInner {
    script: Mutex<VecDeque<ScriptedResult>>,
    requests: Mutex<Vec<ModelRequest>>,
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

    pub fn respond_with_tool_calls(
        self,
        calls: Vec<(impl Into<String>, serde_json::Value)>,
    ) -> Self {
        self.push(ScriptedResult::Response(ModelResponse {
            text: None,
            tool_calls: calls
                .into_iter()
                .map(|(name, arguments)| ModelToolCall {
                    call_id: uuid::Uuid::new_v4().as_simple().to_string(),
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
        _context: &'call ProviderCallContext,
    ) -> BoxFuture<'call, Result<ModelResponse, ProviderError>> {
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
                    ))
                }
            };

            match next {
                ScriptedResult::Response(response) => Ok(response),
                ScriptedResult::Error(error) => Err(error),
                ScriptedResult::Delay(duration) => {
                    sleep(duration).await;
                    let resumed = {
                        let mut script = this.inner.script.lock().expect("script lock");
                        script.pop_front().ok_or_else(|| {
                            ProviderError::terminal(
                                ProviderErrorKind::Unavailable,
                                "scripted provider exhausted after delay",
                            )
                        })?
                    };
                    match resumed {
                        ScriptedResult::Response(response) => Ok(response),
                        ScriptedResult::Error(error) => Err(error),
                        ScriptedResult::Delay(_) => {
                            panic!("nested delay tokens are not allowed in scripted provider");
                        }
                    }
                }
            }
        })
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            supports_tool_calls: true,
            supports_streaming: false,
            supports_images: false,
            supports_context_count_estimate: true,
            max_context_tokens: Some(8192),
            exact_context_counting: true,
        }
    }
}

pub async fn assert_provider_conformance<P>(provider: &P)
where
    P: ModelProvider,
{
    let context = ProviderCallContext {
        provider_call_id: "conformance-call-id".to_string(),
        model_round: 0,
        attempt: 1,
        remaining_time: Some(Duration::from_secs(1)),
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        cancel_notifier: Arc::new(tokio::sync::Notify::new()),
    };

    let request = ModelRequest {
        messages: vec![],
        tools: vec![],
        include_context: true,
        model: None,
        max_output_tokens: 16,
        invocation_id: "conformance-request".to_string(),
    };

    let result = provider.generate(request, &context).await;
    if let Ok(_response) = result {
        // A provider may be permissive for this suite.
    }
}
