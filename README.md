# Agentive

Agentive is a provider-neutral Rust SDK for explicit, testable, and durable agent workflows. It has a local agent loop, deterministic provider tests, typed tools, explicit sub-agent delegation, a subscription-backed Codex runtime, and a Temporal runtime over the same serializable state machine.

**[Read the documentation](https://0x63616c.github.io/agentive/)**

```sh
cargo add agentive agentive-test
```

The smallest run is fully deterministic:

```rust
use agentive::{Agent, RunStatus};
use agentive_test::ScriptedProvider;

# async fn example() -> Result<(), agentive::RunError> {
let agent = Agent::builder()
    .name("greeter")
    .provider(ScriptedProvider::new().respond_with_text("Hello from Agentive."))
    .build()?;

let result = agent.run("Say hello").await?;
assert_eq!(result.status, RunStatus::Completed);
# Ok(())
# }
```

Agentive is under active v0.1 development. [The v1 specification](docs/SPEC.md) is the normative behavior contract; [ADRs](docs/adr/) explain the major boundaries.

Shipped scope includes deterministic `ScriptedProvider` tests, a `#[tool]` macro and manual `Tool` seam, text and image messages, run handles/events/limits/usage, delegation tools, `CodexRuntime`, and `agentive-temporal` with an always-external filesystem payload codec, Codec Server, and durable Temporal workflow/activity adapter.

V1 deliberately has no OpenAI or Anthropic wire adapters, MCP/A2A/AG-UI, approvals, cross-run conversation store, automatic compaction or summarization, S3, generic file/audio/video messages, or payload garbage collection. See [compatibility and security](book/compatibility.md) for the complete boundary.
