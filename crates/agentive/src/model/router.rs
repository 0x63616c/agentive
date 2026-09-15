use super::{
    ModelCapabilities, ModelProvider, ModelRequest, ModelResponse, ModelStream,
    ProviderCallContext, StructuredOutputSupport,
};
use crate::ProviderError;

/// A two-stage provider router whose fallback remains inside Agentive's retry accounting.
///
/// The primary handles attempt one. A retryable, pre-output failure lets the runtime route
/// attempt two (and any explicitly configured later attempts) to the fallback. The router never
/// performs a hidden retry itself.
pub struct FallbackProvider<Primary, Fallback> {
    primary: Primary,
    fallback: Fallback,
}

impl<Primary, Fallback> FallbackProvider<Primary, Fallback> {
    /// Creates a primary-then-fallback routing policy.
    pub fn new(primary: Primary, fallback: Fallback) -> Self {
        Self { primary, fallback }
    }

    /// Borrows the primary provider.
    pub fn primary(&self) -> &Primary {
        &self.primary
    }

    /// Borrows the fallback provider.
    pub fn fallback(&self) -> &Fallback {
        &self.fallback
    }
}

impl<Primary, Fallback> ModelProvider for FallbackProvider<Primary, Fallback>
where
    Primary: ModelProvider,
    Fallback: ModelProvider,
{
    fn routing_attempts(&self) -> u8 {
        2
    }

    fn structured_output_support(&self) -> StructuredOutputSupport {
        weakest_structured_support(
            self.primary.structured_output_support(),
            self.fallback.structured_output_support(),
        )
    }

    fn conservative_context_token_bound(
        &self,
        request: &ModelRequest,
    ) -> Result<Option<u64>, String> {
        common_bound(
            self.primary.conservative_context_token_bound(request)?,
            self.fallback.conservative_context_token_bound(request)?,
        )
    }

    fn exact_context_token_count(&self, request: &ModelRequest) -> Result<Option<u64>, String> {
        common_bound(
            self.primary.exact_context_token_count(request)?,
            self.fallback.exact_context_token_count(request)?,
        )
    }

    async fn generate<'call>(
        &'call self,
        request: ModelRequest,
        context: &'call ProviderCallContext,
    ) -> Result<ModelResponse, ProviderError> {
        if context.attempt == 1 {
            self.primary.generate(request, context).await
        } else {
            self.fallback.generate(request, context).await
        }
    }

    async fn stream<'call>(
        &'call self,
        request: ModelRequest,
        context: &'call ProviderCallContext,
    ) -> Result<ModelStream, ProviderError> {
        if context.attempt == 1 {
            self.primary.stream(request, context).await
        } else {
            self.fallback.stream(request, context).await
        }
    }

    fn capabilities(&self) -> ModelCapabilities {
        guaranteed_capabilities(self.primary.capabilities(), self.fallback.capabilities())
    }
}

fn common_bound(primary: Option<u64>, fallback: Option<u64>) -> Result<Option<u64>, String> {
    Ok(match (primary, fallback) {
        (Some(primary), Some(fallback)) => Some(primary.max(fallback)),
        _ => None,
    })
}

fn weakest_structured_support(
    primary: StructuredOutputSupport,
    fallback: StructuredOutputSupport,
) -> StructuredOutputSupport {
    use StructuredOutputSupport::{Json, JsonSchema, None};
    match (primary, fallback) {
        (JsonSchema, JsonSchema) => JsonSchema,
        (Json | JsonSchema, Json | JsonSchema) => Json,
        _ => None,
    }
}

fn guaranteed_capabilities(
    primary: ModelCapabilities,
    fallback: ModelCapabilities,
) -> ModelCapabilities {
    ModelCapabilities {
        supports_tool_calls: primary.supports_tool_calls && fallback.supports_tool_calls,
        supports_streaming: primary.supports_streaming && fallback.supports_streaming,
        supports_images: primary.supports_images && fallback.supports_images,
        max_context_tokens: match (primary.max_context_tokens, fallback.max_context_tokens) {
            (Some(primary), Some(fallback)) => Some(primary.min(fallback)),
            _ => None,
        },
    }
}
