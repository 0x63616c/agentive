# Codex subscription runtime

```sh
cargo add agentive-codex
```

`CodexRuntime::new()` starts an already authenticated local `codex app-server`. It never accepts, reads, or stores an API key or cached credential.

```rust,ignore
use agentive::{CompiledInstructions, InstructionFragment, Message, ModelRequest};
use agentive_codex::CodexRuntime;

# async fn example() -> Result<(), agentive::ProviderError> {
let runtime = CodexRuntime::new();
let request = ModelRequest {
    instructions: CompiledInstructions { core: InstructionFragment::new("core", 1, "Follow trusted instructions."), agent: None, run: None, features: vec![] },
    messages: vec![Message::user("Summarize this repository")],
    tools: vec![], include_context: true, model: None,
    max_output_tokens: 0,
    output_format: agentive::ModelOutputFormat::Text,
    invocation_id: "docs-example".into(),
};
let _response = runtime.run(request).await?;
# Ok(())
# }
```

App Server currently exposes no lossless final-output-token limit. The adapter
therefore requires `include_context: true` and `max_output_tokens: 0` (no exact
cap) and rejects unsupported constraints before launching the process. Reported
token usage is still preserved in the canonical response.

App Server owns whole-turn orchestration, so `CodexRuntime` does not implement Agentive’s one-call `ModelProvider`. It supports canonical text and URL-image input and experimental correlated tool callbacks. Inline images and non-user history are rejected before process start because they cannot be translated losslessly. Use `start` and `CodexRunHandle::cancel` for interruption.

The live smoke test is deliberately ignored and opt-in. It relies only on an already authenticated local Codex installation, accepts no API key, and does not print account data:

```text
AGENTIVE_CODEX_LIVE=1 cargo test -p agentive-codex live_subscription_text_tool_usage_and_process_reaping -- --ignored
```

See the [conformance record](https://github.com/0x63616c/agentive/blob/main/crates/codex/docs/conformance.md) for the observed capability matrix and protocol boundary.
