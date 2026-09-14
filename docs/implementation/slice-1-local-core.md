# Slice 1: local vertical core

## Outcome

A Rust application can define typed tools, build an immutable agent with a swappable provider, run a bounded multi-turn tool loop locally, observe typed events and usage, and test the exact same behavior deterministically with `ScriptedProvider`.

## Public seams

Tests and callers cross only these seams:

1. `Agent::builder`, `Agent::start`, and `Agent::run` for complete workflow behavior.
2. `ModelProvider` for live and scripted model adapters.
3. Typed `Tool` plus `#[tool]` for directly testable tool authoring.
4. `RunHandle` and its event stream for status, cancellation, result, and observation.
5. `ScriptedProvider` and the provider conformance entry point in `agents-test`.

Internal state transitions, channels, erased traits, retry machinery, schema validators, and task management are implementation details and are not directly tested.

## Workspace delivered by this slice

- `agents`: provider-neutral types, traits, agent runtime, local executor, tools, events, limits, usage, and errors.
- `agents-macros`: first-party `#[tool]` procedural macro.
- `agents-test`: `ScriptedProvider`, request matchers, deterministic helpers, and provider conformance suite.

No provider transport, Temporal, storage, sub-agent, protocol, or documentation-site dependency belongs in these crates during this slice.

## Test-first tracer order

Each item begins as one failing public-behavior test, receives only enough implementation to pass, and is followed by the next test.

1. A scripted single text response completes through `Agent::run` and returns committed history and usage.
2. A scripted tool request invokes one manual typed tool, appends one correlated structured result, and completes on the next provider response.
3. `#[tool]` defines a stateless function with explicit name/description, one typed arguments struct, strict schema, structured output, and direct-call usability.
4. The macro supports a stateful tool implementation and optional borrowed `ToolContext` excluded from schema.
5. Missing descriptions, invalid signatures, invalid names, and unsupported returns produce stable compile failures; duplicate names fail agent construction.
6. Unknown and invalid correlated calls return safe repairable tool results; uncorrelatable protocol fails the run.
7. Tool errors expose only safe code/message. Retry occurs only for an explicitly idempotent tool and retryable error, preserving invocation identity.
8. Multiple calls are sequential by default; explicit parallel-safe calls are bounded and their committed results retain provider order.
9. Model-call limit, elapsed deadline, cancellation, and terminal-race behavior return typed incomplete/failed/cancelled outcomes with execution records.
10. `start` and `RunHandle` expose status, explicit cancellation, result waiting, and a lossless ordered event stream with bounded backpressure; dropping observers never cancels the run.
11. Text and inline/URL image messages compile into canonical requests. Unsupported capabilities and invalid histories fail before provider transport.
12. Instruction layers remain distinct canonically and use deterministic escaped XML only for a provider requiring flattening.
13. `AllOrError` admits an exact or conservative-upper-bound request and rejects unknown/oversized context unless the explicit provider-enforced opt-out is selected.
14. Usage aggregates all reported attempts and preserves unknown fields; hard token budgets use the conservative enforcement ledger.
15. The reusable provider conformance suite passes against `ScriptedProvider` for requests, capability rejection, errors, cancellation, streams, correlation, and usage.

## Acceptance gates

- All tests describe behavior through the five public seams.
- At least one test demonstrates that replacing `ScriptedProvider` with another conforming provider requires no agent or tool changes.
- Macro compile-pass/fail coverage runs under `trybuild`.
- State-machine and untrusted-decoder properties run under `proptest` where examples cannot cover the state space.
- `cargo fmt --all --check`, Clippy with warnings denied, workspace tests, doctests, and rustdoc with warnings denied pass on Rust 1.94 and current stable.
- Core has no HTTP, Codex, Temporal, filesystem-storage, MCP, or OpenTelemetry exporter dependency.
- No unsafe code and no public `anyhow`, `async-trait`, Tokio receiver, or string-keyed extension bag.

## Explicit non-goals

- Codex or any live provider.
- Sub-agents.
- Temporal and External Storage.
- Approvals, conversation storage, memory, retrieval, MCP, or provider-specific options.
- Stabilizing names for functionality not exercised in this slice.
