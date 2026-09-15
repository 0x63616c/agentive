use crate::ScriptedProvider;
use agentive::{
    CompiledInstructions, InstructionFragment, Message, ModelFinishReason, ModelOutputFormat,
    ModelProvider, ModelRequest, ModelResponse, ModelStreamEvent, ModelTokenUsage, ModelToolCall,
    ProviderCallContext, ProviderError, ProviderErrorKind, ProviderToolDescriptor,
    StructuredOutputSupport, ToolName,
};
use futures::StreamExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::Notify;

/// Failure returned by the reusable provider contract suite.
#[derive(Debug, Error)]
pub enum ProviderConformanceError {
    #[error("provider success contract failed: {0}")]
    Success(String),
    #[error("provider error-classification contract failed: {0}")]
    ErrorClassification(String),
    #[error("provider cancellation contract failed: {0}")]
    Cancellation(String),
    #[error("provider streaming contract failed: {0}")]
    Streaming(String),
    #[error("provider structured-output contract failed: {0}")]
    StructuredOutput(String),
    #[error("provider request-preservation contract failed: {0}")]
    RequestPreservation(String),
}

/// Fixtures needed by the common provider adapter contract suite.
///
/// Adapter crates can implement this for a deterministic wire mock and invoke
/// [`assert_provider_conformance`] from their own tests.
pub trait ProviderConformanceHarness {
    type Provider: ModelProvider;

    fn canonical_request(&self) -> ModelRequest;
    fn success_provider(&self, expected: ModelRequest) -> Self::Provider;
    fn classified_error_provider(&self, expected: ModelRequest) -> Self::Provider;
    fn correlated_tool_provider(&self, expected: ModelRequest) -> Self::Provider;
    fn cancellation_provider(&self, expected: ModelRequest) -> Self::Provider;
    fn streaming_provider(&self, expected: ModelRequest) -> Self::Provider;
    fn structured_provider(&self, expected: ModelRequest) -> Self::Provider;
    fn recorded_requests(&self, provider: &Self::Provider) -> Vec<ModelRequest>;
}

/// Runs request, response, error, cancellation, streaming, and structured-output
/// contracts against one provider adapter harness.
pub async fn assert_provider_conformance<H>(harness: &H) -> Result<(), ProviderConformanceError>
where
    H: ProviderConformanceHarness,
{
    let request = harness.canonical_request();

    let capabilities = harness.success_provider(request.clone()).capabilities();
    if !capabilities.supports_tool_calls || capabilities.max_context_tokens.is_none() {
        return Err(ProviderConformanceError::Success(
            "advertised capabilities do not cover the conformance fixtures".to_string(),
        ));
    }
    if !harness
        .streaming_provider(request.clone())
        .capabilities()
        .supports_streaming
    {
        return Err(ProviderConformanceError::Streaming(
            "streaming fixture does not advertise streaming support".to_string(),
        ));
    }

    let provider = harness.success_provider(request.clone());
    let response = provider
        .generate(request.clone(), &context())
        .await
        .map_err(|error| ProviderConformanceError::Success(error.to_string()))?;
    if response.text.as_deref() != Some("conformance-ok")
        || response.usage.as_ref().and_then(|usage| usage.input) != Some(7)
        || !matches!(response.finish_reason, ModelFinishReason::Stop)
    {
        return Err(ProviderConformanceError::Success(
            "response text, usage, or finish reason changed".to_string(),
        ));
    }
    if harness.recorded_requests(&provider) != vec![request.clone()] {
        return Err(ProviderConformanceError::RequestPreservation(
            "canonical request was not passed through exactly once".to_string(),
        ));
    }

    let provider = harness.correlated_tool_provider(request.clone());
    let response = provider
        .generate(request.clone(), &context())
        .await
        .map_err(|error| ProviderConformanceError::Success(error.to_string()))?;
    if !matches!(response.finish_reason, ModelFinishReason::ToolCalls)
        || !matches!(response.tool_calls.as_slice(), [call]
            if call.call_id == "conformance-tool-call"
                && call.name.as_str() == "conformance_tool"
                && call.provider_call_id.as_deref() == Some("conformance-call"))
    {
        return Err(ProviderConformanceError::Success(
            "tool-call correlation was not preserved".to_string(),
        ));
    }

    let provider = harness.classified_error_provider(request.clone());
    let error = provider
        .generate(request.clone(), &context())
        .await
        .expect_err("classified-error fixture must fail");
    if !matches!(error.kind, ProviderErrorKind::RateLimited)
        || !error.is_retryable()
        || !error.is_safe_before_output()
    {
        return Err(ProviderConformanceError::ErrorClassification(
            "stable kind or retry advice was not preserved".to_string(),
        ));
    }

    let provider = harness.cancellation_provider(request.clone());
    let call_context = context();
    let call = provider.generate(request.clone(), &call_context);
    tokio::pin!(call);
    tokio::task::yield_now().await;
    call_context.cancelled.store(true, Ordering::SeqCst);
    call_context.cancel_notifier.notify_waiters();
    if call.await.is_ok() {
        return Err(ProviderConformanceError::Cancellation(
            "provider completed successfully after cancellation".to_string(),
        ));
    }

    let provider = harness.streaming_provider(request.clone());
    let events = provider
        .stream(request.clone(), &context())
        .await
        .map_err(|error| ProviderConformanceError::Streaming(error.to_string()))?
        .collect::<Vec<_>>()
        .await;
    if events.len() != 3
        || !matches!(events.first(), Some(Ok(ModelStreamEvent::TextChunk { text })) if text == "conformance-")
        || !matches!(events.get(1), Some(Ok(ModelStreamEvent::Usage { usage })) if usage.output == Some(2))
        || !matches!(events.last(), Some(Ok(ModelStreamEvent::Done { response })) if response.text.as_deref() == Some("conformance-ok"))
    {
        return Err(ProviderConformanceError::Streaming(
            "ordered chunks, usage, or terminal response changed".to_string(),
        ));
    }

    let mut structured_request = request;
    structured_request.output_format = ModelOutputFormat::JsonSchema {
        schema: serde_json::json!({
            "type": "object",
            "required": ["ok"],
            "properties": {"ok": {"type": "boolean"}}
        }),
    };
    let provider = harness.structured_provider(structured_request.clone());
    if !matches!(
        provider.structured_output_support(),
        StructuredOutputSupport::JsonSchema
    ) {
        return Err(ProviderConformanceError::StructuredOutput(
            "provider did not advertise its fixture guarantee".to_string(),
        ));
    }
    let response = provider
        .generate(structured_request, &context())
        .await
        .map_err(|error| ProviderConformanceError::StructuredOutput(error.to_string()))?;
    let value: serde_json::Value = serde_json::from_str(response.text.as_deref().unwrap_or(""))
        .map_err(|error| ProviderConformanceError::StructuredOutput(error.to_string()))?;
    if value != serde_json::json!({"ok": true}) {
        return Err(ProviderConformanceError::StructuredOutput(
            "structured fixture changed".to_string(),
        ));
    }

    Ok(())
}

/// Reusable harness backed by [`ScriptedProvider`].
#[derive(Debug, Default)]
pub struct ScriptedProviderHarness;

impl ProviderConformanceHarness for ScriptedProviderHarness {
    type Provider = ScriptedProvider;

    fn canonical_request(&self) -> ModelRequest {
        ModelRequest {
            instructions: CompiledInstructions {
                core: InstructionFragment::new("agentive.core", 1, "Be explicit."),
                agent: Some("Conformance agent".to_string()),
                run: Some("Preserve this request".to_string()),
                features: vec![InstructionFragment::new(
                    "agentive.feature.conformance",
                    1,
                    "Exercise the adapter contract.",
                )],
            },
            messages: vec![Message::user("hello")],
            tools: vec![ProviderToolDescriptor {
                name: ToolName::parse("conformance_tool").expect("static valid tool name"),
                description: "Return conformance data.".to_string(),
                schema: serde_json::json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {}
                }),
                idempotent: true,
            }],
            include_context: true,
            model: Some("conformance-model".to_string()),
            max_output_tokens: 64,
            output_format: ModelOutputFormat::Text,
            invocation_id: "conformance-invocation".to_string(),
        }
    }

    fn success_provider(&self, expected: ModelRequest) -> Self::Provider {
        ScriptedProvider::new()
            .expect_request(expected)
            .respond_with(success_response())
    }

    fn classified_error_provider(&self, expected: ModelRequest) -> Self::Provider {
        ScriptedProvider::new()
            .expect_request(expected)
            .respond_with_error(ProviderError::retryable(
                ProviderErrorKind::RateLimited,
                "retry later",
            ))
    }

    fn correlated_tool_provider(&self, expected: ModelRequest) -> Self::Provider {
        ScriptedProvider::new()
            .expect_request(expected)
            .respond_with(ModelResponse {
                text: None,
                tool_calls: vec![ModelToolCall {
                    call_id: "conformance-tool-call".to_string(),
                    name: ToolName::parse("conformance_tool").expect("static valid tool name"),
                    arguments: serde_json::json!({}),
                    provider_call_id: Some("conformance-call".to_string()),
                }],
                usage: Some(ModelTokenUsage::default()),
                finish_reason: ModelFinishReason::ToolCalls,
            })
    }

    fn cancellation_provider(&self, expected: ModelRequest) -> Self::Provider {
        ScriptedProvider::new()
            .expect_request(expected)
            .delay(Duration::from_secs(30))
            .respond_with_text("must not complete")
    }

    fn streaming_provider(&self, expected: ModelRequest) -> Self::Provider {
        ScriptedProvider::new()
            .expect_request(expected)
            .respond_with_stream(vec![
                Ok(ModelStreamEvent::TextChunk {
                    text: "conformance-".to_string(),
                }),
                Ok(ModelStreamEvent::Usage {
                    usage: ModelTokenUsage {
                        output: Some(2),
                        ..ModelTokenUsage::default()
                    },
                }),
                Ok(ModelStreamEvent::Done {
                    response: success_response(),
                }),
            ])
    }

    fn structured_provider(&self, expected: ModelRequest) -> Self::Provider {
        ScriptedProvider::new()
            .expect_request(expected)
            .respond_with_text(r#"{"ok":true}"#)
    }

    fn recorded_requests(&self, provider: &Self::Provider) -> Vec<ModelRequest> {
        provider.recorded_requests()
    }
}

fn context() -> ProviderCallContext {
    ProviderCallContext {
        provider_call_id: "conformance-call".to_string(),
        model_round: 0,
        attempt: 1,
        remaining_time: Some(Duration::from_secs(1)),
        cancelled: Arc::new(AtomicBool::new(false)),
        cancel_notifier: Arc::new(Notify::new()),
    }
}

fn success_response() -> ModelResponse {
    ModelResponse {
        text: Some("conformance-ok".to_string()),
        tool_calls: Vec::new(),
        usage: Some(ModelTokenUsage {
            input: Some(7),
            output: Some(2),
            provider_total: Some(9),
            ..ModelTokenUsage::default()
        }),
        finish_reason: ModelFinishReason::Stop,
    }
}
