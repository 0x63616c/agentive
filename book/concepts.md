# Core concepts

`Agent` owns one stable provider, an immutable per-run tool snapshot, and optional agent instructions. `RunOptions` supplies one-run policy: prior `history`, run instructions, model-call and token limits, output reservation, context-estimation policy, deadline, provider retry ceiling, parallel-tool ceiling, and delegation depth.

Each call produces a `RunResult`: terminal `RunStatus`, optional text, full run-owned history, aggregated `RunUsage`, and provider-call records. A result can be `Completed`, `Incomplete`, `Failed`, or `Cancelled`; only `Completed` is a successful answer.

The model boundary is `ModelProvider`. It receives a canonical `ModelRequest` and `ProviderCallContext` and returns a canonical `ModelResponse`. Implement it only for a provider whose semantics really are one model call; an integration that owns an agent loop belongs behind a separate runtime contract, as `CodexRuntime` does.

There is no global conversation store. Supply previous messages through `RunOptions::history`; the run appends user, assistant, and correlated tool messages. This makes provider context inspectable and portable. Agentive does not invent approval steps: tool access and authorization remain application responsibilities.
