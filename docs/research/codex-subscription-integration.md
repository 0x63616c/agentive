# Codex subscription integration

## Decision

The first deterministic provider is `ScriptedProvider`. The first live target is Codex using the user's ChatGPT subscription. OpenAI API and Anthropic adapters are deferred.

## Verified integration surface

Codex supports signing in with ChatGPT for subscription access as well as API-key authentication. Codex App Server owns its ChatGPT OAuth flow and persists and refreshes the resulting credentials. It exposes a JSON-RPC API centered on threads, turns, streamed items, and turn lifecycle notifications. Its custom dynamic-tool callback surface is currently documented as experimental. The official Codex SDK list currently covers TypeScript and Python, so a Rust integration must speak the App Server protocol directly unless an official Rust SDK appears.

Sources:

- [Codex authentication](https://developers.openai.com/codex/auth)
- [Codex App Server](https://developers.openai.com/codex/app-server)
- [Codex SDK](https://developers.openai.com/codex/sdk)

## Architectural constraint

Codex App Server is an agent runtime, not merely a raw one-model-turn transport. Our `ModelProvider` contract must not conceal a second autonomous loop. The implementation therefore starts as a bounded conformance spike rather than assuming the final type name.

The spike must prove:

1. Existing ChatGPT subscription authentication can be reused without the SDK handling or exposing raw credentials.
2. Text and image inputs can be represented without semantic loss.
3. SDK tool definitions and correlated tool results can cross the App Server boundary while our runtime retains the agreed tool policy and diagnostics.
4. Final text, usage, cancellation, failures, and ordered events can map to canonical SDK types.
5. Parallel SDK runs remain isolated.
6. Codex does not silently invoke capabilities outside the tools and permissions configured by the application.

If every requirement holds without duplicating the agent loop, the integration is `CodexProvider`. If Codex necessarily owns orchestration, it is exposed separately as `CodexRuntime` or `CodexBackend`. That is still a successful subscription integration; it simply represents the real abstraction.

## Deferred

- OpenAI API wire adapter
- Anthropic wire adapter
- MCP
- OpenTelemetry exporter crates
- S3-compatible Storage Driver
- Separate cross-run conversation storage
