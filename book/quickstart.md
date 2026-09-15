# Install and quickstart

```sh
cargo add agentive agentive-test
```

The smallest complete run uses `ScriptedProvider`, which makes the provider response explicit and repeatable:

```rust,ignore
use agentive::{Agent, RunStatus};
use agentive_test::ScriptedProvider;

# async fn example() -> Result<(), agentive::RunError> {
let provider = ScriptedProvider::new().respond_with_text("Hello from Agentive.");
let agent = Agent::builder().name("greeter").provider(provider).build()?;
let result = agent.run("Say hello").await?;
assert_eq!(result.status, RunStatus::Completed);
assert_eq!(result.text.as_deref(), Some("Hello from Agentive."));
# Ok(())
# }
```

Run it inside a Tokio runtime. `Agent::run` is sugar over `Agent::start(...).wait()`: it compiles trusted instructions and complete history, calls the provider, executes requested tools, appends correlated tool results, and stops on final text, cancellation, failure, or a configured limit. Use [the Codex runtime](codex.md) for a real local subscription turn, or [Temporal](temporal.md) when the same effect/state loop must survive worker restarts.
