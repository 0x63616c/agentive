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

Use `respond_with_tool_calls` followed by a final response for multi-round tool behavior. Inspect `recorded_requests()` when request shape is a contract, and use `assert_provider_conformance` for provider implementations. `ScriptedProvider` is the same public `ModelProvider` seam used by real providers, not a separate loop-only mock.

Test public behavior: completed histories, safe repair messages for unknown tools and invalid arguments, event order, cancellation, limits, usage, retry identity, and child attribution. Live Codex smoke tests are intentionally opt-in; see [Codex runtime](codex.md).
