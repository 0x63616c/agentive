# Agentive

Agentive is an idiomatic, provider-neutral Rust foundation for explicit, testable agent workflows. It provides a typed local agent loop, typed tools, deterministic tests, a local Codex subscription runtime, and an always-external Temporal payload pipeline.

```sh
cargo add agentive-sdk --rename agentive
cargo add agentive-test
```

The [quickstart](quickstart.md) is the smallest useful run. This site documents shipped behavior and its explicit boundaries.

## What is shipped

- `agentive-sdk` (imported as `agentive`): local agent loop, message model, tools, run handles, events, limits, and usage.
- `agentive-test`: deterministic `ScriptedProvider` and provider conformance support.
- `agentive-codex`: a locally authenticated `codex app-server` runtime.
- `agentive-temporal`: always-external payload encoding, filesystem and in-memory storage, codecs, Codec Server, Temporal data conversion, and a durable workflow/activity runtime.

The public API is v0.1.0. Read [compatibility and security](compatibility.md) before production deployment.
