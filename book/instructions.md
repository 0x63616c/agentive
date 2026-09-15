# Explicit instruction layers

Agentive compiles instruction layers in a fixed order: the versioned SDK core fragment, optional `AgentBuilder::agent_instructions`, optional `RunOptions::run_instructions`, and runtime feature fragments such as tool-result handling. These are trusted instruction layers; user messages and retrieved/external content never become instructions.

`CompiledInstructions::render_xml()` produces deterministic escaped XML for transports that require flattened instructions. Do not concatenate untrusted text into a privileged layer.

```rust,ignore
use agentive::RunOptions;

let options = RunOptions {
    run_instructions: Some("Answer in three bullet points.".into()),
    ..RunOptions::default()
};
assert_eq!(options.model_call_limit, 6);
```

The core layer treats user messages and external content as untrusted. Put application behavior and authorization rules in your own trusted agent or run layer. V1 has no approval or authorization-policy API, so registering a tool is the application's authorization decision.
