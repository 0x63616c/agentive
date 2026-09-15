use crate::errors::ProviderError;
use crate::model::{
    ModelCapabilities, ModelProvider, ModelRequest, ModelResponse, ProviderCallContext,
};
use futures::future::BoxFuture;
use std::sync::Arc;

/// Object-safe provider boundary used by the runtime.
pub trait DynModelProvider: Send + Sync {
    /// Provider capabilities.
    fn capabilities(&self) -> ModelCapabilities;
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
    fn capabilities(&self) -> ModelCapabilities {
        self.0.capabilities()
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
) -> Result<ModelResponse, ProviderError> {
    if provider.capabilities().supports_streaming {
        let mut stream = provider.stream(request.clone(), context).await?;
        while let Some(event) = futures::StreamExt::next(&mut stream).await {
            match event? {
                crate::model::ModelStreamEvent::Done { response } => return Ok(response),
                crate::model::ModelStreamEvent::Usage { .. }
                | crate::model::ModelStreamEvent::TextChunk { .. }
                | crate::model::ModelStreamEvent::ToolCallChunk { .. } => {}
            }
        }
        return Err(ProviderError::terminal(
            crate::errors::ProviderErrorKind::Protocol,
            "provider stream ended without a terminal response",
        ));
    }
    provider.generate(request, context).await
}
