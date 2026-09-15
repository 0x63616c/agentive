# agentive-codex

`agentive-codex` drives a locally authenticated `codex app-server` process using
the user's existing Codex subscription. It never accepts, reads, or persists an
API key or cached credential.

## Boundary

Codex App Server owns agent turn orchestration. Accordingly this crate exposes
[`CodexRuntime`] rather than implementing Agentive's one-model-turn
`ModelProvider` trait. It starts an isolated ephemeral App Server thread per
run, translates canonical text and URL-image input, streams text deltas, and
returns canonical `ModelResponse` and `ModelTokenUsage` values.

Dynamic tools use App Server's experimental, correlated `item/tool/call`
callback. Registered canonical `Tool` objects receive a canonical
`ToolContext`, including the run cancellation token; unknown or malformed calls
receive a safe protocol response. Inline images and non-user history are
rejected before process start because their current App Server representations
are not lossless.

See [the App Server conformance record](docs/conformance.md) for the observed
capability matrix and opt-in live smoke-test command.

## Example

```no_run
use agentive::{CompiledInstructions, InstructionFragment, Message, ModelRequest};
use agentive_codex::CodexRuntime;

# async fn example() -> Result<(), agentive::ProviderError> {
let runtime = CodexRuntime::new();
let request = ModelRequest {
    instructions: CompiledInstructions { core: InstructionFragment::new("core", 1, "Follow trusted application instructions."), agent: None, run: None, features: vec![] },
    messages: vec![Message::user("Summarize this repository")],
    tools: vec![],
    include_context: true,
    model: None,
    max_output_tokens: 0,
    output_format: agentive::ModelOutputFormat::Text,
    invocation_id: "example".to_owned(),
};
let response = runtime.run(request).await?;
println!("{}", response.text.unwrap_or_default());
# Ok(())
# }
```

The current App Server protocol has no lossless final-output-token cap, so this
runtime requires `include_context: true` and `max_output_tokens: 0` (explicitly
uncapped). Unsupported constraints fail before a process is opened; usage is
still returned when App Server reports it.
