# Providers and deterministic mocks

`ModelProvider` is the one-turn provider boundary. It receives an owned canonical `ModelRequest` plus a borrowed `ProviderCallContext`, and returns a canonical `ModelResponse`. The context carries the stable provider-call identity, model round, attempt number, remaining time, and cancellation state; it is runtime metadata, not serialized model content.

```rust,ignore
use agentive::{ModelProvider, ModelRequest, ModelResponse, ProviderCallContext, ProviderError};

struct EchoProvider;

impl ModelProvider for EchoProvider {
    fn generate<'call>(
        &'call self,
        request: ModelRequest,
        context: &'call ProviderCallContext,
    ) -> impl std::future::Future<Output = Result<ModelResponse, ProviderError>> + Send + 'call {
        async move {
            if context.is_cancelled() {
                return Err(ProviderError::terminal(
                    agentive::ProviderErrorKind::Timeout,
                    "provider call was cancelled",
                ));
            }
            Ok(ModelResponse {
                text: Some(format!("received {} message(s)", request.messages.len())),
                tool_calls: vec![],
                usage: None,
                finish_reason: agentive::ModelFinishReason::Stop,
            })
        }
    }
}
```

Capabilities are a contract. `ModelCapabilities` declares tool, streaming, image, and context-admission support. If a requested canonical capability cannot be preserved, reject it explicitly; adapters must not silently discard images, schemas, instructions, or tool correlation. `stream` is optional on the trait and returns an owned `ModelStream` when supported.

The deterministic `agentive-test::ScriptedProvider` implements this same public trait. Script responses, tool calls, delays, and classified failures; then assert the whole script is consumed and inspect `recorded_requests()` when prompt or schema shape matters:

```rust,ignore
use agentive::{Agent, RunStatus};
use agentive_test::ScriptedProvider;

# async fn example() -> Result<(), agentive::RunError> {
let provider = ScriptedProvider::new().respond_with_text("done");
let agent = Agent::builder().provider(provider.clone()).build()?;
let result = agent.run("finish the task").await?;
assert_eq!(result.status, RunStatus::Completed);
provider.assert_finished();
assert_eq!(provider.recorded_requests().len(), 1);
# Ok(())
# }
```

Use `ProviderError::terminal` or `ProviderError::retryable` with a classified `ProviderErrorKind`. The runtime owns bounded, cancellation-aware retry decisions; a provider attempts one transport operation and must not independently replay an ambiguous request.
