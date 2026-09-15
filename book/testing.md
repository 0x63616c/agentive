# Deterministic testing

`ScriptedProvider` tests complete agent behavior without a network, account, or timing race:

```rust,ignore
use agentive::{Agent, RunStatus};
use agentive_test::ScriptedProvider;

# async fn example() -> Result<(), agentive::RunError> {
let provider = ScriptedProvider::new().respond_with_text("finished");
let agent = Agent::builder().provider(provider.clone()).build()?;
let result = agent.run("do the thing").await?;
assert_eq!(result.status, RunStatus::Completed);
provider.assert_finished();
# Ok(())
# }
```

Use `respond_with_tool_calls` followed by a final response for multi-round tool behavior. Use `respond_with_stream` for ordered native chunks and the terminal `ModelStreamEvent::Done`. `expect_request` asserts the complete canonical request before returning the next scripted result; `recorded_requests()` supports less rigid inspection.

Every provider adapter should implement `ProviderConformanceHarness` around its deterministic wire mock and invoke `assert_provider_conformance`. The shared suite verifies exact request preservation, response usage, classified errors and retry advice, cancellation, ordered native streaming, and structured output. `ScriptedProviderHarness` proves the reference mock against the same contract. `ScriptedProvider` is the public `ModelProvider` seam used by real providers, not a separate loop-only mock.

Test public behavior: completed histories, safe repair messages for unknown tools and invalid arguments, event order, cancellation, limits, usage, retry identity, and child attribution. Live Codex smoke tests are intentionally opt-in; see [Codex runtime](codex.md).
