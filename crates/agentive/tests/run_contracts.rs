#![allow(
    missing_docs,
    clippy::expect_used,
    clippy::manual_async_fn,
    clippy::unwrap_used
)]

use agentive::{
    Agent, ModelCapabilities, ModelFinishReason, ModelProvider, ModelResponse, ModelStreamEvent,
    ProviderCallContext, ProviderError, ProviderErrorKind, RunOptions, RunStatus,
};
use agentive_test::ScriptedProvider;
use futures::StreamExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema, PartialEq)]
struct StructuredAnswer {
    answer: String,
}

#[tokio::test]
async fn run_model_selection_reaches_the_canonical_provider_request() {
    let provider = ScriptedProvider::new().respond_with_text("done");
    let probe = provider.clone();
    let agent = Agent::builder().provider(provider).build().expect("agent");

    agent
        .run_with_options(
            "start",
            RunOptions {
                model: Some("provider/model-v1".to_string()),
                ..RunOptions::default()
            },
        )
        .await
        .expect("run");

    assert_eq!(
        probe.recorded_requests()[0].model.as_deref(),
        Some("provider/model-v1")
    );
}

#[tokio::test]
async fn blank_model_selection_fails_before_provider_transport() {
    let provider = ScriptedProvider::new().respond_with_text("must not run");
    let probe = provider.clone();
    let agent = Agent::builder().provider(provider).build().expect("agent");

    let error = agent
        .run_with_options(
            "start",
            RunOptions {
                model: Some("  ".to_string()),
                ..RunOptions::default()
            },
        )
        .await
        .expect_err("blank model");

    assert!(matches!(error, agentive::RunError::Config(_)));
    assert!(probe.recorded_requests().is_empty());
}

#[tokio::test]
async fn typed_structured_output_is_declared_before_transport_and_retains_the_run() {
    let provider = ScriptedProvider::new().respond_with_text(r#"{"answer":"forty-two"}"#);
    let probe = provider.clone();
    let agent = Agent::builder().provider(provider).build().expect("agent");

    let result = agent
        .run_structured::<StructuredAnswer>("answer")
        .await
        .expect("typed response");

    assert_eq!(result.value.answer, "forty-two");
    assert_eq!(result.run.status, RunStatus::Completed);
    assert!(matches!(
        probe.recorded_requests()[0].output_format,
        agentive::ModelOutputFormat::JsonSchema { .. }
    ));
}

#[tokio::test]
async fn unsupported_structured_output_fails_before_provider_transport() {
    let provider = ScriptedProvider::new()
        .structured_output_support(agentive::StructuredOutputSupport::None)
        .respond_with_text(r#"{"answer":"must not run"}"#);
    let probe = provider.clone();
    let agent = Agent::builder().provider(provider).build().expect("agent");

    let error = agent
        .run_structured::<StructuredAnswer>("answer")
        .await
        .expect_err("unsupported schema must fail");

    assert!(matches!(error, agentive::RunError::Capability(_)));
    assert!(probe.recorded_requests().is_empty());
}

#[tokio::test]
async fn scripted_provider_consumes_native_stream_chunks() {
    let response = ModelResponse {
        text: Some("done".into()),
        tool_calls: Vec::new(),
        usage: None,
        finish_reason: ModelFinishReason::Stop,
    };
    let provider = ScriptedProvider::new()
        .capabilities(ModelCapabilities {
            supports_streaming: true,
            max_context_tokens: Some(8_192),
            ..ModelCapabilities::default()
        })
        .respond_with_stream(vec![
            Ok(ModelStreamEvent::TextChunk { text: "do".into() }),
            Ok(ModelStreamEvent::TextChunk { text: "ne".into() }),
            Ok(ModelStreamEvent::Done { response }),
        ]);
    let agent = Agent::builder().provider(provider).build().expect("agent");

    let result = agent.run("start").await.expect("streamed run");

    assert_eq!(result.text.as_deref(), Some("done"));
}

#[tokio::test]
async fn run_sugar_does_not_backpressure_on_an_unobserved_attached_event_buffer() {
    let response = ModelResponse {
        text: Some("done".into()),
        tool_calls: Vec::new(),
        usage: None,
        finish_reason: ModelFinishReason::Stop,
    };
    let mut events = (0..300)
        .map(|_| Ok(ModelStreamEvent::TextChunk { text: "x".into() }))
        .collect::<Vec<_>>();
    events.push(Ok(ModelStreamEvent::Done { response }));
    let provider = ScriptedProvider::new()
        .capabilities(ModelCapabilities {
            supports_streaming: true,
            max_context_tokens: Some(8_192),
            ..ModelCapabilities::default()
        })
        .respond_with_stream(events);
    let agent = Agent::builder().provider(provider).build().expect("agent");

    let result = tokio::time::timeout(Duration::from_secs(1), agent.run("start"))
        .await
        .expect("run sugar must detach its unused event buffer")
        .expect("streamed run");

    assert_eq!(result.text.as_deref(), Some("done"));
}

#[tokio::test]
async fn streaming_usage_is_committed_without_becoming_user_visible_output() {
    let usage = agentive::ModelTokenUsage {
        input: Some(2),
        output: Some(1),
        cached_input: Some(0),
        reasoning: Some(0),
        provider_total: Some(3),
    };
    let response = ModelResponse {
        text: Some("done".into()),
        tool_calls: Vec::new(),
        usage: None,
        finish_reason: ModelFinishReason::Stop,
    };
    let provider = ScriptedProvider::new()
        .capabilities(ModelCapabilities {
            supports_streaming: true,
            max_context_tokens: Some(8_192),
            ..ModelCapabilities::default()
        })
        .respond_with_stream(vec![
            Ok(ModelStreamEvent::Usage {
                usage: usage.clone(),
            }),
            Err(ProviderError::retryable(
                ProviderErrorKind::Transport,
                "retry before output",
            )),
        ])
        .respond_with_stream(vec![
            Ok(ModelStreamEvent::Usage {
                usage: usage.clone(),
            }),
            Ok(ModelStreamEvent::Done { response }),
        ]);
    let agent = Agent::builder().provider(provider).build().expect("agent");

    let result = agent
        .run_with_options(
            "start",
            RunOptions {
                provider_max_attempts: 2,
                ..RunOptions::default()
            },
        )
        .await
        .expect("usage-only first stream remains retryable");

    assert_eq!(result.usage.model_calls.len(), 2);
    assert_eq!(result.usage.aggregate.provider_total, None);
    assert_eq!(result.usage.model_calls[1].usage.provider_total, Some(3));
}

#[tokio::test]
async fn wait_retains_a_result_when_the_run_finishes_before_subscription() {
    let agent = Agent::builder()
        .provider(ScriptedProvider::new().respond_with_text("done"))
        .build()
        .expect("agent");
    let handle = agent.start("start", RunOptions::default());

    tokio::time::sleep(Duration::from_millis(20)).await;
    let result = tokio::time::timeout(Duration::from_secs(1), handle.wait())
        .await
        .expect("retained result must be observable")
        .expect("run result");

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.text.as_deref(), Some("done"));
}

#[tokio::test]
async fn scripted_text_response_completes() {
    use agentive::{Agent, RunStatus};
    use agentive_test::ScriptedProvider;

    let provider = ScriptedProvider::new().respond_with_text("Done");
    let agent = Agent::builder().provider(provider).build().unwrap();

    let result = agent.run("run this").await.unwrap();

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.text, Some("Done".to_string()));
    assert_eq!(result.records.len(), 1);
    assert!(
        result.records[0]
            .response
            .as_ref()
            .is_some_and(|response| response.text.is_some())
    );
    assert_eq!(result.history.len(), 2);
}

#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct EchoArgs {
    #[schemars(description = "text to echo back to the model")]
    text: String,
}

struct EchoTool;

impl agentive::Tool for EchoTool {
    fn name(&self) -> &agentive::ToolName {
        static NAME: std::sync::OnceLock<agentive::ToolName> = std::sync::OnceLock::new();
        NAME.get_or_init(|| agentive::ToolName::parse("echo").expect("tool name must be valid"))
    }

    fn description(&self) -> &'static str {
        "Echo text"
    }

    fn schema_json(&self) -> &serde_json::Value {
        use schemars::schema_for;
        static SCHEMA: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| serde_json::to_value(schema_for!(EchoArgs)).unwrap())
    }

    fn idempotent(&self) -> bool {
        false
    }

    fn call(
        &self,
        _context: &agentive::ToolContext,
        args: serde_json::Value,
    ) -> impl std::future::Future<Output = Result<serde_json::Value, agentive::ToolError>> + Send + '_
    {
        async move {
            let args: EchoArgs = agentive::decode_tool_call_args(args).map_err(|err| {
                agentive::ToolError::terminal("invalid_arguments", err.to_string())
            })?;
            Ok(serde_json::json!({ "output": args.text }))
        }
    }
}

#[tokio::test]
async fn scripted_tool_call_halts_and_completes() {
    use agentive::{Agent, RunStatus};
    use agentive_test::ScriptedProvider;

    let provider = ScriptedProvider::new()
        .respond_with_tool_calls(vec![("echo", serde_json::json!({"text": "payload"}))])
        .respond_with_text("echoed");

    let agent = Agent::builder()
        .provider(provider)
        .tool(EchoTool)
        .build()
        .unwrap();

    let result = agent.run("please echo").await.unwrap();

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.text, Some("echoed".to_string()));
    assert_eq!(result.history.len(), 4);
    assert_eq!(result.history[1].tool_calls.as_ref().unwrap().len(), 1);
    assert_eq!(result.history[2].role, agentive::MessageRole::Tool);
    assert_eq!(
        result.history[2].tool_call_id,
        Some(result.history[1].tool_calls.as_ref().unwrap()[0].id.clone())
    );
}

#[tokio::test]
async fn unknown_tool_returns_safe_error_to_model() {
    use agentive::{Agent, MessageContent, RunStatus};
    use agentive_test::ScriptedProvider;

    let provider = ScriptedProvider::new()
        .respond_with_tool_calls(vec![("missing", serde_json::json!({"x": 1}))])
        .respond_with_text("finished");

    let agent = Agent::builder().provider(provider).build().unwrap();

    let result = agent.run("please fail").await.unwrap();

    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.text, Some("finished".to_string()));
    assert_eq!(result.history.len(), 4);
    let tool_message = &result.history[2];
    assert_eq!(tool_message.role, agentive::MessageRole::Tool);
    let MessageContent::Text { text } = &tool_message.content[0] else {
        panic!("expected text tool result")
    };
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!({"error": {"code": "tool_not_found", "message": "tool not found"}})
    );
}

#[tokio::test]
async fn start_and_cancel_marks_cancelled() {
    use agentive::{Agent, RunOptions, RunStatus};
    use agentive_test::ScriptedProvider;
    use std::time::Duration;
    use tokio::time::sleep;

    let provider = ScriptedProvider::new()
        .delay(Duration::from_millis(100))
        .respond_with_text("late");
    let agent = Agent::builder().provider(provider).build().unwrap();

    let handle = agent.start("slow", RunOptions::default());
    sleep(Duration::from_millis(20)).await;
    handle.cancel();

    let result = tokio::time::timeout(Duration::from_millis(50), handle.wait())
        .await
        .expect("cancellation must interrupt the provider attempt")
        .unwrap();
    assert_eq!(result.status, RunStatus::Cancelled);
    assert!(result.text.is_none());
    assert_eq!(result.history, vec![agentive::Message::user("slow")]);
}

#[derive(Clone)]
struct NonCooperativeProvider;

impl ModelProvider for NonCooperativeProvider {
    fn exact_context_token_count(
        &self,
        request: &agentive::ModelRequest,
    ) -> Result<Option<u64>, String> {
        serde_json::to_vec(request)
            .map(|rendered| Some(u64::try_from(rendered.len()).unwrap_or(u64::MAX)))
            .map_err(|error| format!("could not render test model request: {error}"))
    }

    async fn generate(
        &self,
        _request: agentive::ModelRequest,
        _context: &ProviderCallContext,
    ) -> Result<ModelResponse, ProviderError> {
        futures::future::pending().await
    }
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            max_context_tokens: Some(8192),
            ..Default::default()
        }
    }
}

#[tokio::test]
async fn deadline_interrupts_a_non_cooperative_provider() {
    use agentive::{Agent, RunOptions, RunStatus};

    let agent = Agent::builder()
        .provider(NonCooperativeProvider)
        .build()
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        agent.run_with_options(
            "slow",
            RunOptions {
                deadline: Some(Duration::from_millis(10)),
                ..Default::default()
            },
        ),
    )
    .await
    .expect("deadline must stop an uncooperative provider")
    .unwrap();
    assert_eq!(result.status, RunStatus::Cancelled);
}

struct NonCooperativeTool;
impl agentive::Tool for NonCooperativeTool {
    fn name(&self) -> &agentive::ToolName {
        static NAME: std::sync::OnceLock<agentive::ToolName> = std::sync::OnceLock::new();
        NAME.get_or_init(|| "hang".parse().unwrap())
    }
    fn description(&self) -> &'static str {
        "never completes"
    }
    fn schema_json(&self) -> &serde_json::Value {
        static SCHEMA: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "additionalProperties": false,
            })
        })
    }
    fn call(
        &self,
        _: &agentive::ToolContext,
        _: serde_json::Value,
    ) -> impl std::future::Future<Output = Result<serde_json::Value, agentive::ToolError>> + Send + '_
    {
        async { futures::future::pending().await }
    }
}

#[tokio::test]
async fn deadline_interrupts_a_non_cooperative_tool() {
    use agentive::{Agent, RunOptions, RunStatus};
    use agentive_test::ScriptedProvider;

    let agent = Agent::builder()
        .provider(
            ScriptedProvider::new().respond_with_tool_calls(vec![("hang", serde_json::json!({}))]),
        )
        .tool(NonCooperativeTool)
        .build()
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        agent.run_with_options(
            "slow tool",
            RunOptions {
                deadline: Some(Duration::from_millis(10)),
                ..Default::default()
            },
        ),
    )
    .await
    .expect("deadline must stop an uncooperative tool")
    .unwrap();
    assert_eq!(result.status, RunStatus::Cancelled);
}

#[derive(Clone)]
struct StreamingProvider {
    calls: Arc<AtomicUsize>,
}
impl ModelProvider for StreamingProvider {
    fn exact_context_token_count(
        &self,
        request: &agentive::ModelRequest,
    ) -> Result<Option<u64>, String> {
        serde_json::to_vec(request)
            .map(|rendered| Some(u64::try_from(rendered.len()).unwrap_or(u64::MAX)))
            .map_err(|error| format!("could not render test model request: {error}"))
    }

    async fn generate(
        &self,
        _: agentive::ModelRequest,
        _: &ProviderCallContext,
    ) -> Result<ModelResponse, ProviderError> {
        unreachable!("streaming capability selects stream")
    }
    fn stream<'call>(
        &'call self,
        _: agentive::ModelRequest,
        _: &'call ProviderCallContext,
    ) -> impl std::future::Future<
        Output = Result<
            futures::stream::BoxStream<'static, Result<ModelStreamEvent, ProviderError>>,
            ProviderError,
        >,
    > + Send
    + 'call {
        self.calls.fetch_add(1, Ordering::SeqCst);
        async {
            Ok(Box::pin(futures::stream::iter(vec![
                Ok(ModelStreamEvent::TextChunk { text: "hel".into() }),
                Ok(ModelStreamEvent::TextChunk { text: "lo".into() }),
                Ok(ModelStreamEvent::Done {
                    response: ModelResponse {
                        text: Some("hello".into()),
                        tool_calls: vec![],
                        usage: None,
                        finish_reason: ModelFinishReason::Stop,
                    },
                }),
            ]))
                as futures::stream::BoxStream<
                    'static,
                    Result<ModelStreamEvent, ProviderError>,
                >)
        }
    }
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            supports_streaming: true,
            max_context_tokens: Some(8192),
            ..Default::default()
        }
    }
}

#[tokio::test]
async fn streaming_provider_emits_native_deltas_in_order() {
    use agentive::{Agent, RunEvent};
    let agent = Agent::builder()
        .provider(StreamingProvider {
            calls: Arc::new(AtomicUsize::new(0)),
        })
        .build()
        .unwrap();
    let handle = agent.start("stream", Default::default());
    let events = handle.events();
    futures::pin_mut!(events);
    let result = handle.wait().await.unwrap();
    let events: Vec<_> = events.take(6).collect().await;
    assert_eq!(result.text.as_deref(), Some("hello"));
    assert!(
        matches!(&events[2], RunEvent::ModelDelta { event: ModelStreamEvent::TextChunk { text }, .. } if text == "hel")
    );
    assert!(
        matches!(&events[3], RunEvent::ModelDelta { event: ModelStreamEvent::TextChunk { text }, .. } if text == "lo")
    );
}

struct VisibleFailureProvider {
    calls: Arc<AtomicUsize>,
}
impl ModelProvider for VisibleFailureProvider {
    fn exact_context_token_count(
        &self,
        request: &agentive::ModelRequest,
    ) -> Result<Option<u64>, String> {
        serde_json::to_vec(request)
            .map(|rendered| Some(u64::try_from(rendered.len()).unwrap_or(u64::MAX)))
            .map_err(|error| format!("could not render test model request: {error}"))
    }

    async fn generate(
        &self,
        _: agentive::ModelRequest,
        _: &ProviderCallContext,
    ) -> Result<ModelResponse, ProviderError> {
        unreachable!("streaming capability selects stream")
    }
    fn stream<'call>(
        &'call self,
        _: agentive::ModelRequest,
        _: &'call ProviderCallContext,
    ) -> impl std::future::Future<
        Output = Result<
            futures::stream::BoxStream<'static, Result<ModelStreamEvent, ProviderError>>,
            ProviderError,
        >,
    > + Send
    + 'call {
        self.calls.fetch_add(1, Ordering::SeqCst);
        async {
            Ok(Box::pin(futures::stream::iter(vec![
                Ok(ModelStreamEvent::TextChunk {
                    text: "visible".into(),
                }),
                Err(ProviderError::retryable(
                    ProviderErrorKind::Transport,
                    "lost after output",
                )),
            ]))
                as futures::stream::BoxStream<
                    'static,
                    Result<ModelStreamEvent, ProviderError>,
                >)
        }
    }
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            supports_streaming: true,
            max_context_tokens: Some(8192),
            ..Default::default()
        }
    }
}

#[tokio::test]
async fn streaming_failure_after_visible_delta_is_not_retried() {
    use agentive::{Agent, RunOptions, RunStatus};
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = Agent::builder()
        .provider(VisibleFailureProvider {
            calls: Arc::clone(&calls),
        })
        .build()
        .unwrap();
    let result = agent
        .run_with_options(
            "stream",
            RunOptions {
                provider_max_attempts: 2,
                ..Default::default()
            },
        )
        .await
        .expect("terminal provider failure is retained as a run result");
    assert_eq!(result.status, RunStatus::Failed);
    assert_eq!(result.records[0].attempts.len(), 1);
    assert!(result.records[0].error.is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unsupported_images_are_rejected_before_provider_transport() {
    use agentive::{Agent, Message, RunError, RunOptions};
    use agentive_test::ScriptedProvider;

    let provider = ScriptedProvider::new().respond_with_text("must not run");
    let agent = Agent::builder().provider(provider.clone()).build().unwrap();
    let error = agent
        .run_with_options(
            "describe",
            RunOptions {
                history: vec![
                    Message::image_url("https://example.test/image.png", "image/png")
                        .expect("valid image"),
                ],
                ..Default::default()
            },
        )
        .await
        .expect_err("unsupported image");
    assert!(matches!(error, RunError::Capability(message) if message.contains("images")));
    assert!(provider.recorded_requests().is_empty());
}
