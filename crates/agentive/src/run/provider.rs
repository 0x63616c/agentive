use super::{RunEvent, RunObserver};
use crate::errors::ProviderError;
use crate::model::{
    ModelCapabilities, ModelProvider, ModelRequest, ModelResponse, ProviderCallContext,
    StructuredOutputSupport,
};
use futures::future::BoxFuture;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Object-safe provider boundary used by the runtime.
pub trait DynModelProvider: Send + Sync {
    /// Minimum attempts required by provider routing policy.
    fn routing_attempts(&self) -> u8;
    /// Provider capabilities.
    fn capabilities(&self) -> ModelCapabilities;
    /// Structured-output support guaranteed by the provider.
    fn structured_output_support(&self) -> StructuredOutputSupport;
    /// Proven upper bound for the provider's rendered prompt tokens.
    fn conservative_context_token_bound(
        &self,
        request: &ModelRequest,
    ) -> Result<Option<u64>, String>;
    /// Exact input-token count for this provider's selected model, excluding reserve.
    fn exact_context_token_count(&self, request: &ModelRequest) -> Result<Option<u64>, String>;
    /// Generate a complete response.
    fn generate<'call>(
        &'call self,
        request: ModelRequest,
        context: &'call ProviderCallContext,
    ) -> BoxFuture<'call, Result<ModelResponse, ProviderError>>;
    /// Generate a streamed response.
    fn stream<'call>(
        &'call self,
        request: ModelRequest,
        context: &'call ProviderCallContext,
    ) -> BoxFuture<'call, Result<crate::model::ModelStream, ProviderError>>;
}

pub(super) struct ProviderErased<T>(T);
impl<T> ProviderErased<T> {
    pub(super) fn new(inner: T) -> Self {
        Self(inner)
    }
}
impl<T: ModelProvider> DynModelProvider for ProviderErased<T> {
    fn routing_attempts(&self) -> u8 {
        self.0.routing_attempts()
    }
    fn capabilities(&self) -> ModelCapabilities {
        self.0.capabilities()
    }
    fn structured_output_support(&self) -> StructuredOutputSupport {
        self.0.structured_output_support()
    }
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
    ) -> BoxFuture<'call, Result<ModelResponse, ProviderError>> {
        Box::pin(self.0.generate(request, context))
    }
    fn stream<'call>(
        &'call self,
        request: ModelRequest,
        context: &'call ProviderCallContext,
    ) -> BoxFuture<'call, Result<crate::model::ModelStream, ProviderError>> {
        Box::pin(self.0.stream(request, context))
    }
}

pub(super) async fn execute_provider_effect(
    provider: &Arc<dyn DynModelProvider>,
    request: ModelRequest,
    context: &ProviderCallContext,
    round: u32,
    observer: Option<&RunObserver>,
    visible_delta: &AtomicBool,
) -> Result<ModelResponse, ProviderError> {
    if provider.capabilities().supports_streaming {
        let mut stream = provider.stream(request.clone(), context).await?;
        let mut streamed_usage = None;
        while let Some(event) = futures::StreamExt::next(&mut stream).await {
            let event = event?;
            match event {
                crate::model::ModelStreamEvent::Done { mut response } => {
                    if let (Some(streamed), Some(final_usage)) = (&streamed_usage, &response.usage)
                        && streamed != final_usage
                    {
                        return Err(ProviderError::terminal(
                            crate::errors::ProviderErrorKind::Protocol,
                            "provider stream reported conflicting usage",
                        ));
                    }
                    if response.usage.is_none() {
                        response.usage = streamed_usage;
                    }
                    return Ok(response);
                }
                crate::model::ModelStreamEvent::Usage { usage } => {
                    streamed_usage = Some(usage.clone());
                    if let Some(observer) = observer {
                        observer
                            .emit(RunEvent::ModelDelta {
                                round,
                                event: crate::model::ModelStreamEvent::Usage { usage },
                            })
                            .await;
                    }
                }
                event @ (crate::model::ModelStreamEvent::TextChunk { .. }
                | crate::model::ModelStreamEvent::ToolCallChunk { .. }) => {
                    visible_delta.store(true, Ordering::SeqCst);
                    if let Some(observer) = observer {
                        observer.emit(RunEvent::ModelDelta { round, event }).await;
                    }
                }
            }
        }
        return Err(ProviderError::terminal(
            crate::errors::ProviderErrorKind::Protocol,
            "provider stream ended without a terminal response",
        ));
    }
    provider.generate(request, context).await
}
