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

/// Provider-reported token usage for one response.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelTokenUsage {
    /// Input tokens.
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cached_input: Option<u64>,
    pub reasoning: Option<u64>,
    pub provider_total: Option<u64>,
}

/// Required precision for context admission.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AllOrError {
    /// Require exact provider counting.
    Exact,
    Conservative,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderToolDescriptor {
    pub name: ToolName,
    pub description: String,
    pub schema: Value,
    pub idempotent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelRequest {
    /// Canonical SDK, agent, run, and active feature instruction layers for this round.
    pub instructions: CompiledInstructions,
    pub messages: Vec<Message>,
    pub tools: Vec<ProviderToolDescriptor>,
    pub include_context: bool,
    pub model: Option<String>,
    pub max_output_tokens: u64,
    pub invocation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFinishReason {
    Stop,
    ToolCalls,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelToolCall {
    pub call_id: String,
    pub name: ToolName,
    pub arguments: Value,
    pub provider_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelResponse {
    pub text: Option<String>,
    pub tool_calls: Vec<ModelToolCall>,
    pub usage: Option<ModelTokenUsage>,
    pub finish_reason: ModelFinishReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStreamEvent {
    TextChunk {
        text: String,
    },
    ToolCallChunk {
        call_id: String,
        name: ToolName,
        arguments_fragment: String,
    },
    Usage {
        usage: ModelTokenUsage,
    },
    Done {
        response: ModelResponse,
    },
}

pub type ModelStream = BoxStream<'static, Result<ModelStreamEvent, crate::errors::ProviderError>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub supports_tool_calls: bool,
    pub supports_streaming: bool,
    pub supports_images: bool,
    pub supports_context_count_estimate: bool,
    pub max_context_tokens: Option<u64>,
    pub exact_context_counting: bool,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            supports_tool_calls: true,
            supports_streaming: false,
            supports_images: false,
            supports_context_count_estimate: true,
            max_context_tokens: None,
            exact_context_counting: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderCallContext {
    pub provider_call_id: String,
    pub model_round: u32,
    pub attempt: u8,
    pub remaining_time: Option<std::time::Duration>,
    pub cancelled: std::sync::Arc<AtomicBool>,
    pub cancel_notifier: std::sync::Arc<Notify>,
}

impl ProviderCallContext {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

pub trait ModelProvider: Send + Sync + 'static {
    fn generate<'call>(
        &'call self,
        request: ModelRequest,
        context: &'call ProviderCallContext,
    ) -> impl Future<Output = Result<ModelResponse, crate::errors::ProviderError>> + Send + 'call;

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

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities::default()
    }
}

#[derive(Debug, Default, Clone)]
pub struct UsageEstimator;

impl UsageEstimator {
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

    pub fn can_admit(
        &self,
        mode: AllOrError,
        messages: &[Message],
        capabilities: &ModelCapabilities,
        output_token_reserve: u64,
        provider_enforced_limit_opt_out: bool,
    ) -> Result<(), String> {
        if provider_enforced_limit_opt_out {
            return Ok(());
        }

        let Some(limit) = capabilities.max_context_tokens else {
            return Err("provider context limit unknown".to_string());
        };

        if !capabilities.supports_context_count_estimate {
            return Err("provider context estimation unsupported".to_string());
        }

        if mode == AllOrError::Exact && !capabilities.exact_context_counting {
            return Err("exact context estimation unavailable".to_string());
        }

        let required = self.estimate_tokens(messages, mode, output_token_reserve);
        if required > limit {
            return Err(format!(
                "required {required} tokens exceeds provider limit {limit}",
            ));
        }
        Ok(())
    }

    pub fn has_exact_counting(&self, capabilities: &ModelCapabilities) -> bool {
        capabilities.exact_context_counting
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
