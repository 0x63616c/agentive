use crate::ids::ToolName;
use crate::message::CompiledInstructions;
use crate::message::Message;
use crate::usage::TokenUsage;
use futures::stream::BoxStream;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

use serde::{Deserialize, Serialize};
use serde_json::Value;

mod router;
pub use router::FallbackProvider;

/// Provider-reported token usage for one response.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ModelTokenUsage {
    /// Input tokens.
    pub input: Option<u64>,
    /// Generated output tokens.
    pub output: Option<u64>,
    /// Input tokens served from a provider cache.
    pub cached_input: Option<u64>,
    /// Tokens used for model reasoning, when reported.
    pub reasoning: Option<u64>,
    /// Provider-reported total token count.
    pub provider_total: Option<u64>,
}

/// Required precision for context admission.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AllOrError {
    /// Require exact provider counting.
    Exact,
    /// Accept a proven conservative provider bound.
    Conservative,
}

/// Tool metadata rendered for a model provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderToolDescriptor {
    /// Stable declared tool name.
    pub name: ToolName,
    /// Model-visible explanation of the tool.
    pub description: String,
    /// JSON Schema accepted by the tool.
    pub schema: Value,
    /// Whether repeated execution is safe under the tool contract.
    pub idempotent: bool,
}

/// Structured-output capability guaranteed by a provider for every eligible request.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutputSupport {
    /// Only ordinary text output is supported.
    #[default]
    None,
    /// Valid JSON may be requested, without an exact schema guarantee.
    Json,
    /// An exact JSON Schema may be requested.
    JsonSchema,
}

/// Canonical output contract requested from a provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelOutputFormat {
    /// Ordinary provider text.
    #[default]
    Text,
    /// Any valid JSON value.
    Json,
    /// JSON conforming to the supplied schema.
    JsonSchema {
        /// Schema that the JSON response must satisfy.
        schema: Value,
    },
}

/// Complete provider-neutral input for one model call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelRequest {
    /// Canonical SDK, agent, run, and active feature instruction layers for this round.
    pub instructions: CompiledInstructions,
    /// Conversation history for the call.
    pub messages: Vec<Message>,
    /// Tools the provider may request.
    pub tools: Vec<ProviderToolDescriptor>,
    /// Whether provider context should be included.
    pub include_context: bool,
    /// Optional provider model selector.
    pub model: Option<String>,
    /// Maximum requested response tokens; zero requests no exact provider-side cap.
    pub max_output_tokens: u64,
    /// Required provider output contract.
    #[serde(default)]
    pub output_format: ModelOutputFormat,
    /// Stable runtime identity for this invocation.
    pub invocation_id: String,
}

/// Reason a provider ended a model response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFinishReason {
    /// The provider completed ordinary output.
    Stop,
    /// The provider requested one or more tools.
    ToolCalls,
    /// The provider terminated with an error description.
    Error(String),
}

/// A tool call emitted by a model provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelToolCall {
    /// Stable call identifier in the model response.
    pub call_id: String,
    /// Declared tool name.
    pub name: ToolName,
    /// JSON tool arguments.
    pub arguments: Value,
    /// Provider-native call identifier, when available.
    pub provider_call_id: Option<String>,
}

/// A completed response from a model provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelResponse {
    /// Optional generated text.
    pub text: Option<String>,
    /// Tool calls requested by the provider.
    pub tool_calls: Vec<ModelToolCall>,
    /// Provider-reported usage, when available.
    pub usage: Option<ModelTokenUsage>,
    /// Why the response ended.
    pub finish_reason: ModelFinishReason,
}

/// An event produced while streaming a model response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStreamEvent {
    /// A generated text fragment.
    TextChunk {
        /// Fragment text.
        text: String,
    },
    /// A fragment of a streamed tool call.
    ToolCallChunk {
        /// Stable model call identifier.
        call_id: String,
        /// Declared tool name.
        name: ToolName,
        /// Partial JSON arguments text.
        arguments_fragment: String,
    },
    /// Provider-reported usage information.
    Usage {
        /// Usage associated with the stream.
        usage: ModelTokenUsage,
    },
    /// The terminal assembled response.
    Done {
        /// Completed provider response.
        response: ModelResponse,
    },
}

/// A boxed asynchronous stream of provider model events.
pub type ModelStream = BoxStream<'static, Result<ModelStreamEvent, crate::errors::ProviderError>>;

/// Provider capabilities relevant to request admission and execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilities {
    /// Whether the provider supports tool calls.
    pub supports_tool_calls: bool,
    /// Whether the provider supports streamed responses.
    pub supports_streaming: bool,
    /// Whether the provider accepts image messages.
    pub supports_images: bool,
    /// Maximum combined prompt and output tokens, when known.
    pub max_context_tokens: Option<u64>,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            supports_tool_calls: true,
            supports_streaming: false,
            supports_images: false,
            max_context_tokens: None,
        }
    }
}

/// Runtime context supplied to a single provider call.
#[derive(Debug, Clone)]
pub struct ProviderCallContext {
    /// Stable identity for the provider-side call.
    pub provider_call_id: String,
    /// Zero-based model round within the invocation.
    pub model_round: u32,
    /// One-based runtime attempt number.
    pub attempt: u8,
    /// Time remaining for the call, when bounded.
    pub remaining_time: Option<std::time::Duration>,
    /// Shared cancellation state.
    pub cancelled: std::sync::Arc<AtomicBool>,
    /// Notification triggered when cancellation is requested.
    pub cancel_notifier: std::sync::Arc<Notify>,
}

impl ProviderCallContext {
    /// Returns whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Provider adapter used to generate or stream model responses.
pub trait ModelProvider: Send + Sync + 'static {
    /// Minimum runtime-owned attempts required by this provider's routing policy.
    fn routing_attempts(&self) -> u8 {
        1
    }
    /// Returns the strongest structured-output contract this provider guarantees.
    fn structured_output_support(&self) -> StructuredOutputSupport {
        StructuredOutputSupport::None
    }
    /// Returns a proven upper bound for this provider's rendered prompt tokens.
    ///
    /// The returned value excludes [`ModelRequest::max_output_tokens`]. Returning
    /// `None` refuses conservative admission because the provider cannot prove a
    /// bound for its wire rendering.
    fn conservative_context_token_bound(
        &self,
        _request: &ModelRequest,
    ) -> Result<Option<u64>, String> {
        Ok(None)
    }

    /// Counts prompt tokens using this provider's selected model and wire format.
    ///
    /// The returned value excludes [`ModelRequest::max_output_tokens`]. Returning
    /// `None` declares that this provider cannot supply an exact count for this
    /// request. Capability metadata alone is never treated as an exact counter.
    fn exact_context_token_count(&self, _request: &ModelRequest) -> Result<Option<u64>, String> {
        Ok(None)
    }

    /// Generates one complete response for the request.
    fn generate<'call>(
        &'call self,
        request: ModelRequest,
        context: &'call ProviderCallContext,
    ) -> impl Future<Output = Result<ModelResponse, crate::errors::ProviderError>> + Send + 'call;

    /// Starts a stream of response events for the request.
    fn stream<'call>(
        &'call self,
        _request: ModelRequest,
        _context: &'call ProviderCallContext,
    ) -> impl Future<Output = Result<ModelStream, crate::errors::ProviderError>> + Send + 'call
    {
        async {
            let stream: ModelStream = Box::pin(futures::stream::empty::<
                Result<ModelStreamEvent, crate::errors::ProviderError>,
            >());
            Ok(stream)
        }
    }

    /// Returns the provider capabilities used for admission and routing.
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities::default()
    }
}

/// Helper for token admission checks and legacy message estimates.
#[derive(Debug, Default, Clone)]
pub struct UsageEstimator;

impl UsageEstimator {
    /// Returns a conservative prompt-token bound for the complete canonical request.
    ///
    /// The canonical request is UTF-8 JSON and every token consumes at least one
    /// byte, so its byte length is an upper bound for that representation. This is
    /// deliberately a bound rather than a tokenizer estimate. Provider adapters
    /// use this helper only when their wire rendering is this canonical form.
    pub fn conservative_context_token_bound(&self, request: &ModelRequest) -> Result<u64, String> {
        serde_json::to_vec(request)
            .map(|rendered| u64::try_from(rendered.len()).unwrap_or(u64::MAX))
            .map_err(|error| format!("could not render complete model request: {error}"))
    }

    /// Estimates tokens from canonical messages when a provider bound is unavailable.
    pub fn estimate_tokens(
        &self,
        messages: &[Message],
        mode: AllOrError,
        output_token_reserve: u64,
    ) -> u64 {
        let base = messages
            .iter()
            .flat_map(|m| m.content.iter())
            .map(|content| match content {
                crate::message::MessageContent::Text { text } => text.len() as u64 / 4 + 1,
                crate::message::MessageContent::Image { .. } => 64,
            })
            .sum::<u64>();

        let required = base.saturating_add(output_token_reserve);
        match mode {
            AllOrError::Exact => required,
            AllOrError::Conservative => required.saturating_mul(2),
        }
    }

    /// Verifies that a request and its output reserve fit the provider context limit.
    ///
    /// Returns an error when the provider cannot prove the requested admission mode.
    pub fn can_admit(
        &self,
        mode: AllOrError,
        request: &ModelRequest,
        capabilities: &ModelCapabilities,
        provider_prompt_token_bound: Option<u64>,
        provider_enforced_limit_opt_out: bool,
    ) -> Result<(), String> {
        if provider_enforced_limit_opt_out {
            return Ok(());
        }

        let Some(limit) = capabilities.max_context_tokens else {
            return Err("provider context limit unknown".to_string());
        };

        let prompt_tokens = match mode {
            AllOrError::Exact => provider_prompt_token_bound
                .ok_or_else(|| "exact provider/model context counter unavailable".to_string())?,
            AllOrError::Conservative => provider_prompt_token_bound
                .ok_or_else(|| "conservative provider context bound unavailable".to_string())?,
        };
        let required = prompt_tokens.saturating_add(request.max_output_tokens);
        if required > limit {
            return Err(format!(
                "required {required} tokens exceeds provider limit {limit}",
            ));
        }
        Ok(())
    }
}

impl From<TokenUsage> for ModelTokenUsage {
    fn from(value: TokenUsage) -> Self {
        Self {
            input: value.input,
            output: value.output,
            cached_input: value.cached_input,
            reasoning: value.reasoning,
            provider_total: value.provider_total,
        }
    }
}
