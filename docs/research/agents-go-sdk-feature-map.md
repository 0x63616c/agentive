# Agent SDK for Go: feature map for an idiomatic Rust SDK

## Research baseline

This map is pinned to `agenticenv/agent-sdk-go` **v0.3.6**, commit [`d6a718f`](https://github.com/agenticenv/agent-sdk-go/tree/d6a718fac0c6d116f7264717bb5158255ba6ff61), dated 14 September 2026. The snapshot contains 367 Go files, 110 test files, 1,416 test functions, about 63,445 lines under `pkg/` and `internal/`, and a further 10,443 lines of documentation. It is a product-sized reference implementation, not a small loop to transliterate.

The package-by-package evidence ledger is maintained in [Upstream traceability](./upstream-traceability.md).

The accepted compatibility rule is **behavioral parity through an idiomatic Rust API**. The Rust SDK should preserve useful capabilities and observable semantics while rejecting Go-specific package structure, functional options, channel APIs, context-value plumbing, and generated mock patterns.

## Executive finding

The reference SDK is really six layers:

1. A provider-neutral model protocol.
2. A deterministic agent loop over messages, model calls, tool calls, and final output.
3. Run control and a typed event protocol.
4. Optional capabilities: approvals, hooks, conversation, memory, retrieval, MCP, A2A, and sub-agents.
5. Pluggable execution environments: in-process, locally durable, Temporal, and Restate.
6. Product support: provider adapters, observability, built-in tools, CLI, examples, benchmarks, and eval harnesses.

Trying to stabilize all six layers in a single initial release would make the public API depend on immature integration details. The recommended plan is to stabilize the provider protocol, tool protocol, agent loop, run result, and streaming events first; put integrations behind separate crates and feature gates; and define conformance suites before multiplying implementations.

## Capability inventory

### 1. Agent construction and configuration

The Go `Agent` is the public façade. Construction validates configuration, initializes capability registries and clients, creates a runtime, and exposes run, stream, reconnect, worker, and A2A entry points. Its configuration covers identity, prompt, provider, sampling, response format, tools, execution mode, approval policy and handler, iteration and time limits, budget, conversation, memory, retrieval, sub-agents, MCP, A2A, hooks, observability, runtime selection, and workflow fingerprint behavior. See the [agent configuration source](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/pkg/agent/config.go) and [public Agent implementation](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/pkg/agent/agent.go).

Rust requirements:

- A validating `AgentBuilder`, with required fields enforced at `build()` rather than dozens of functional options.
- Immutable built agent configuration so concurrent runs cannot observe half-applied changes.
- Explicit per-run `RunOptions`; do not use ambient task-local values as the primary API.
- Provider, tool, storage, and runtime extension points that can hold heterogeneous implementations.
- Clear shutdown semantics for owned background resources.
- A small prelude; advanced integration types remain in their owning crates.

### 2. Provider-neutral model protocol

The reference `LLMClient` supports non-streaming generation, streaming generation, model/provider identity, and a streaming-support flag. Requests contain a system message, structured-output request, tool schemas, conversation messages, sampling controls, and provider-neutral reasoning controls. Responses contain text, metadata, token usage, and zero or more tool calls. Streaming chunks carry text deltas, reasoning deltas, and accumulated tool calls. See the [LLM interface](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/pkg/interfaces/llm.go).

Rust requirements:

- A `ModelProvider` trait with asynchronous completion and streaming operations.
- One canonical request/response/message/tool-call model owned by the core crate.
- Typed capability discovery instead of independent booleans and documentation promises.
- Provider-neutral sampling fields with explicit unsupported-field behavior.
- Typed token usage covering input, output, cached input, and reasoning tokens.
- Structured output as an explicit request contract, including schema support level.
- Provider-specific metadata available without making core behavior depend on untyped maps.
- Provider errors classified at least as authentication, rate limit, timeout, transport, invalid request, unavailable, protocol, and unknown.
- Preservation of provider-native error source and retry hints.

Built-in Go adapters are OpenAI, Anthropic, Gemini, DeepSeek, and Ollama. Their feature support is not uniform: sampling knobs differ; DeepSeek and Ollama ignore JSON Schema while supporting JSON mode; reasoning configuration and reporting differ; and model-level tool support can vary. The reference matrix is documented in [LLM Providers](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/getting-started/llm-providers.mdx).

### 3. Mockable provider and conformance kit

Mockability is a first-class requirement, not a side effect of using a trait.

Recommended deliverables:

- `ScriptedProvider`: consumes an ordered script of expected requests and responses, stream chunks, delays, and failures.
- Request capture and structural matchers so tests can assert the exact history and tool schema sent on each iteration.
- Deterministic scripted tool calls, usage reports, reasoning deltas, malformed chunks, and mid-stream errors.
- A reusable provider conformance suite that every built-in and third-party adapter can run.
- Wire-level adapter tests against local mock HTTP servers.
- Opt-in live smoke tests, excluded from ordinary CI, for real credentials and API drift.
- No generated mocks required for normal user tests.

The reference project itself validates orchestration under load with a mock provider and mock tools, and uses mocks throughout its interfaces and runtimes. See its [benchmark harness](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/benchmarks/README.md) and [interface mocks](https://github.com/agenticenv/agent-sdk-go/tree/d6a718fac0c6d116f7264717bb5158255ba6ff61/pkg/interfaces/mocks).

### 4. Agent loop

The core lifecycle is:

1. Validate the run request and establish run identity.
2. Load conversation history when configured.
3. Recall long-term memory when configured.
4. Prefetch retrieval context in prefetch or hybrid mode.
5. Assemble the provider request from system prompt, contextual messages, conversation, user input, and tool schemas.
6. Call the provider, optionally as a stream.
7. Accumulate usage and enforce budget.
8. If the response contains tool calls, authorize, approve if needed, execute, append assistant/tool messages, and repeat.
9. If no tool calls remain, finalize the response.
10. Persist conversation and memory, emit the terminal event, and complete the handle.

Within one local run, the reference passes the complete accumulated in-memory message slice to every model iteration. After a model requests tools, it appends the assistant tool-call message and the resulting tool messages, then resends that enlarged slice on the next model call. Across runs, configured conversation history is loaded once at run start and prepended to the new user message. The default load limit is the 20 most recent messages; the in-memory store retains up to 100 by default. This is count-based truncation rather than token-aware selection or semantic compaction, and there is no general payload-codec layer in the reference implementation.

The reference loop also supports parallel or sequential execution of multiple tool calls, per-operation retries/timeouts, hooks around operations, forced final generation without tools after the iteration limit, sub-agent delegation, and telemetry aggregation. See the [architecture request lifecycle](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/architecture.mdx) and [local loop](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/internal/runtime/local/agent_loop.go).

Rust requirements:

- One runtime-independent loop/state machine used by all execution backends.
- Cancellation-safe boundaries and an explicit policy for partial tool batches.
- Stable ordering of tool results even when execution is concurrent.
- Bounded iterations and time, with typed termination reasons.
- Explicit retry classification; never retry arbitrary tool side effects by default.
- Stable idempotency metadata for tool invocations.
- Deterministic tests with injected IDs, time, provider scripts, and tool scripts where needed.
- No hidden recursion for sub-agents; enforce a depth bound.

### 5. Tools

The Go `Tool` contract exposes name, display name, description, JSON Schema parameters, and asynchronous-by-context execution over an untyped argument map. Optional interfaces add human approval and programmatic authorization. Tool metadata includes a stable idempotency key based on run, iteration, and provider tool-call identity. Tools can be registered statically or through a dynamic registry. See the [tool interface](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/pkg/interfaces/tool.go) and [tool documentation](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/features/tools.mdx).

Rust requirements:

- An object-safe `Tool` trait for heterogeneous registries.
- A typed authoring path that derives or supplies JSON Schema and deserializes arguments into a Rust input type.
- A low-level dynamic JSON path for protocol adapters such as MCP.
- Structured tool output that distinguishes model-visible content, machine data, and optional UI metadata.
- `ToolContext` with run ID, call ID, iteration, cancellation, and idempotency key.
- Optional authorization and approval policies at the orchestration layer.
- Duplicate-name validation and deterministic registry snapshots per run.

Built-in reference tools include calculator, current time, echo, random, search, weather, and Wikipedia. These are examples or optional crates, not reasons to enlarge the core API.

### 6. Runs, handles, status, cancellation, and reconnect

The Go runtime starts work immediately and returns a per-run handle. Both normal and streaming handles expose a stable ID, status, explicit cancellation, blocking result retrieval, and completion notification. Stream handles also expose approval and event subscription. Reconnect can recover handles by run ID when the runtime supports it. Cancelling a caller’s wait does not cancel the underlying run; cancellation is an explicit control action. See the [runtime contracts](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/internal/runtime/runtime.go).

Rust requirements:

- `RunHandle` and `StreamHandle` as owned, cloneable control handles where the backend permits it.
- Separate “stop waiting” from “cancel the run.” Dropping a future must not silently cancel durable work.
- Status enum: pending, running, completed, failed, cancelled.
- Typed `RunId`, plus backend capability reporting for reconnect and replay.
- An event `Stream`, not a Tokio receiver exposed as the public contract.
- Explicit replay cursors rather than assuming every backend has integer offsets.

### 7. Streaming and AG-UI events

The reference emits AG-UI-shaped lifecycle, step, text, tool-call, reasoning, raw, and custom events. Events have timestamps and optional non-wire replay offsets. Supported kinds include `RUN_STARTED`, `RUN_FINISHED`, `RUN_ERROR`, step start/end, text start/content/end, tool-call start/args/end/result, reasoning start/message start/content/message end/end, raw, and custom. See the [event registry](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/internal/events/events.go) and [AG-UI guide](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/features/ag-ui-protocol.mdx).

Rust requirements:

- A non-exhaustive typed `AgentEvent` enum with serde wire compatibility.
- Validated event ordering and terminal-event invariants.
- Streaming fallback when a provider lacks native streaming.
- Backpressure and slow-consumer behavior defined in the contract.
- Replay cursor and delivery semantics defined per runtime.
- Protocol translation isolated from the internal state machine so AG-UI evolution does not destabilize orchestration.

### 8. Approvals and authorization

The reference can gate native tools, MCP calls, A2A calls, sub-agent delegation, and budget continuation. Its policies are require-all, auto-approve, or allowlist. A rejection becomes a model-visible refusal and the agent continues; an approval timeout fails the run. Streaming clients receive custom approval events and respond through the stream handle. See [Approvals](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/features/approvals.mdx).

Rust requirements:

- Distinguish authorization (programmatic permission) from approval (human decision).
- Typed approval request kinds rather than custom untyped payloads internally.
- Exactly-once resolution with explicit already-resolved and expired errors.
- Safe default decided before v0.1 API freeze.
- Approval tokens treated as capabilities and omitted from logs by default.

### 9. Budgets and usage

The Go SDK aggregates usage across every model round and nested sub-agents. Per-run token and USD limits can stop a run or pause for approval. Price inputs are configured manually. See [Budget control](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/features/budget.mdx) and [Token usage](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/features/token-usage.mdx).

Rust requirements:

- Strongly typed counters and money representation; avoid binary floating point for enforced cost limits.
- Parent-owned aggregation across a run tree.
- Explicit behavior when providers omit or revise usage.
- Stop and approval-continuation policies.
- Budget checks after every billable response and before avoidable follow-up work.

### 10. Hooks and guardrails

Hook groups wrap model, tool, retrieval, and memory operations. They may inspect, mutate, or abort. Intended uses include guardrails, privacy filtering, budget tracking, audit logging, tenant isolation, query rewriting, and reranking. See [Hooks](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/features/hooks.mdx).

Rust requirements:

- A deliberately small middleware surface.
- Defined ordering, mutation visibility, short-circuit behavior, and error composition.
- Prefer typed layer/middleware composition over a separate before/after trait for every operation.
- Keep telemetry observers from mutating business inputs.

### 11. Conversation

Conversation persists multi-turn message history under a caller-supplied conversation ID. The reference supplies in-memory and Redis backends, history limits, save-per-iteration behavior, and a distributed-storage capability check. See the [conversation interface](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/pkg/interfaces/conversation.go) and [Conversation guide](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/features/conversation.mdx).

Rust requirements:

- A `ConversationStore` trait with append, load, and clear.
- Stable `ConversationId` and ordered message representation.
- Atomicity expectations for concurrent turns documented and tested.
- In-memory implementation in the test/support crate; Redis as an optional integration.

### 12. Long-term memory

Memory stores scoped records with kind, metadata, expiration, timestamps, relevance score, and stable ID. Scope can combine user, tenant, agent, and arbitrary tags. Recall injects a labeled context message. Store modes include on-demand via a tool and always/extraction after a run. Reference backends are pgvector and Weaviate. See the [memory interface](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/pkg/interfaces/memory.go) and [Memory guide](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/features/memory.mdx).

Rust requirements:

- Keep memory distinct from conversation history and generic retrieval.
- Typed, composable isolation scope.
- TTL and delete/clear semantics enforced by the store contract.
- Extraction prompts and policies configurable outside the storage trait.
- Backend crates optional; no vector database dependency in core.

### 13. Retrieval

Retrievers return ranked documents. Modes are agentic (search exposed as a tool), prefetch (query before the first model call), and hybrid (both). Reference backends are pgvector and Weaviate. See the [retriever interface](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/pkg/interfaces/retriever.go) and [Retrieval guide](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/features/retrieval.mdx).

Rust requirements:

- A small `Retriever` trait returning typed documents.
- Retrieval policy—prefetch, agentic, or hybrid—belongs to orchestration, not storage.
- Source, score, content, and metadata preserved.
- Multi-retriever ordering, partial failure, and deduplication behavior specified.

### 14. MCP

MCP servers are connected at construction, health-checked, queried for tools, filtered, and projected into the agent tool namespace. The reference supports stdio and streamable HTTP, configuration-created clients and supplied clients, retries, and dynamic registration. Names are prefixed `mcp_<server>_<tool>`. See [MCP](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/features/mcp.mdx).

Rust requirements:

- Optional integration crate built on the official or selected Rust MCP SDK.
- MCP tools adapted into the same dynamic tool contract as local tools.
- Collision-safe stable names, connection ownership, graceful close, filtering, and failure policy.
- Transport-specific configuration outside core.

### 15. Sub-agents

Sub-agents are local SDK agents exposed to a parent as delegation tools. Each may have its own provider, prompt, tools, and runtime. Results become tool results for the parent. Temporal maps delegation to child workflows; Restate maps it to a service invocation; local executes in process. Parent usage and budgets aggregate the whole tree. See [Sub-agents](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/features/sub-agents.mdx).

Rust requirements:

- Heterogeneous providers across the agent tree.
- Explicit depth limit and cycle detection at build time where possible.
- Parent-child run IDs and cancellation propagation.
- Defined event fan-in ordering and budget attribution.
- Delegation authorization/approval shares the capability policy model.

### 16. A2A

The reference can expose an agent as an A2A JSON-RPC server and consume remote A2A agents as tool providers. It discovers agent cards and skills, supports messages, streaming, asynchronous task management, authentication, and skill filtering. See [A2A](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/features/a2a.mdx).

Rust requirements:

- Optional protocol crate, separate from local sub-agents.
- Client skills adapted into tools.
- Server adapter over core run/stream handles.
- Authentication and transport policy owned by the application-facing adapter.

### 17. Execution policies

The reference has separate timeout and maximum-attempt controls for model, tool authorization, tool execution, MCP, A2A, retrieval, memory, conversation, and sub-agent operations, with exponential backoff defaults. See [Execution Config](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/features/execution-config.mdx).

Rust requirements:

- A common operation policy type with timeout, retry count, backoff, and retry classifier.
- Conservative defaults that do not retry non-idempotent actions merely because the transport failed.
- Cancellation takes precedence over retry.
- Runtime integrations translate this policy without changing its meaning.

### 18. Execution runtimes and durability

The Go SDK offers:

- In-process execution, now durable by default through a local disk journal.
- Temporal workflows and activities, embedded or split workers, reconnect, stream replay, signals for approvals, child workflows for sub-agents, configuration fingerprints, and continue-as-new handling.
- Restate durable invocations, embedded endpoint, event log, approvals, reconnect, and sub-agent services.

See the [runtime comparison](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/architecture.mdx), [durability semantics](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/advanced/durable-execution.mdx), and [deterministic execution guide](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/advanced/deterministic-execution.mdx).

Rust requirements:

- Start with a correct Tokio in-process runtime.
- Define a runtime-neutral serializable state machine before implementing durability.
- Keep Temporal/Restate/local journal adapters in separate crates.
- Specify step identity, replay behavior, side-effect boundaries, versioning/fingerprints, and upgrade compatibility before claiming durable execution.
- Do not make “drop” imply successful durable shutdown.

### 19. Observability

The reference wraps OpenTelemetry traces, metrics, and logs behind its own interfaces and reports a typed telemetry payload in every result. It records model, tool, memory, retrieval, sub-agent, and run metrics. See [observability interfaces](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/pkg/interfaces/observability.go) and [observability docs](https://github.com/agenticenv/agent-sdk-go/tree/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/observability).

Rust requirements:

- `tracing` instrumentation in core, with OpenTelemetry export as an integration.
- Stable span names and typed fields where interoperability matters.
- Secrets, prompts, tool arguments, approval tokens, and reasoning excluded by default.
- Run result usage/telemetry remains useful without an exporter.

### 20. Supporting product surface

The upstream also contains:

- `agctl` CLI with run, chat, config, and version commands.
- Runnable examples for every major capability.
- A mock-driven concurrency benchmark recording p50/p95/p99 latency, allocations, CPU, usage, success, and storage operations.
- Promptfoo and DeepEval harnesses in CI.
- Security scanning, lint, tests, coverage, and build gates.

These are important quality signals, but the CLI and third-party evaluation frameworks should consume the SDK rather than define its core API. See the [CI workflow](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/.github/workflows/ci.yml) and [eval harness](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/eval-harness/README.md).

## Recommended Rust workspace boundary

Names are provisional; the boundary is the recommendation.

| Layer | Suggested crate | Responsibility |
|---|---|---|
| Stable public core | `agentive` | Agent builder, messages, provider/tool traits, loop, run handles, events, errors, policies |
| Test support | `agentive-test` | Scripted provider/tool/store, matchers, conformance suites, deterministic helpers |
| Providers | `agents-openai`, `agents-anthropic`, etc. | Wire translation only |
| Protocols | `agents-mcp`, `agents-a2a`, `agents-agui` | External protocol adapters |
| Storage | `agents-redis`, `agents-pgvector`, etc. | Conversation, memory, and retrieval backends |
| Durability | `agentive-temporal`, `agentive-restate`, optional local journal | Runtime adapters over the shared state machine |
| Telemetry | `agents-otel` | OpenTelemetry export and semantic mapping |
| Developer tool | `agents-cli` | Optional CLI built only from public APIs |

Do not create every crate on day one. Establish the workspace boundary and add a crate only with its first working integration.

## Recommended delivery slices

### Foundation / first usable release

- Core message, tool-call, response, usage, structured-output, and error types.
- Object-safe swappable provider contract.
- Scripted mock provider and provider conformance suite.
- Typed and dynamic tool authoring.
- Correct multi-iteration loop with sequential and parallel tool execution.
- Agent builder and validation.
- Run result, cancellation, status, and typed event stream.
- Reasoning/event passthrough and structured output.
- Time/iteration limits, idempotency metadata, and safe retry defaults.
- Extensive unit, integration, compile-fail/API, and property-style protocol tests.
- One production provider adapter plus one OpenAI-compatible adapter path.

### Workflow SDK release

- Hooks/guardrails.
- Approvals and authorization, deliberately deferred from the first usable release without speculative v1 approval interfaces.
- Usage budgets.
- Conversation store.
- Sub-agents with heterogeneous providers.
- MCP client integration.
- `tracing` instrumentation and OpenTelemetry adapter.
- Provider adapter matrix expanded.

### Knowledge and interoperability release

- Retrieval modes and storage adapters.
- Long-term memory policies and backends.
- AG-UI wire adapter.
- A2A client/server.

### Durable execution release

- Serializable runtime-neutral state machine.
- Local journal with recovery tests.
- Temporal and/or Restate adapters.
- Reconnect, event replay, approval recovery, versioning, and crash fault-injection tests.

### Product hardening

- CLI, comprehensive examples, benchmark harness, eval harness, MSRV policy, semver checks, supply-chain/security gates, and release automation.

## Test architecture

The minimum polished test pyramid should be:

| Level | Purpose |
|---|---|
| Pure unit tests | State transitions, validation, message conversion, usage aggregation, budgets, backoff, event ordering |
| Scripted integration tests | Full agent loops with mock provider, mock tools, errors, streaming, cancellation, approvals, sub-agents |
| Conformance suites | Same behavioral contract run against every provider, runtime, store, and protocol adapter |
| Wire tests | HTTP/SSE/JSON fixtures and local mock servers for each provider/protocol adapter |
| Compile tests | Public ergonomics, trait implementability, `Send + Sync`, feature combinations, and intentional compile failures |
| Property/fuzz tests | Event decoding, schema/value round trips, malformed provider output, replay cursors, state-machine invariants |
| Fault injection | Mid-stream disconnect, cancellation races, duplicated events, tool timeout, process crash, replay, approval recovery |
| Live smoke tests | Opt-in credentialed checks for API drift, never required for ordinary contributors |
| Benchmarks | Agent-loop overhead, concurrent runs, stream fan-out, tool batches, memory growth |

Every public adapter should ship with its conformance proof. Coverage percentage is useful but is not the acceptance criterion; the acceptance criterion is that the behavioral matrix is exercised.

## Rust-specific architectural pressure

Runtime provider swapping and heterogeneous sub-agents require type erasure somewhere. Native `async fn` in traits is not dyn-compatible, according to the [Rust Reference](https://doc.rust-lang.org/reference/items/traits.html#dyn-compatibility). A provider contract therefore needs either boxed futures/streams, an `async-trait`-style transformation, or a generic public API plus an internal erased adapter. The generic-only design becomes invasive once tools, provider routers, sub-agents, and dynamic configuration must coexist.

The accepted direction is a native `ModelProvider` authoring trait plus a cloneable `DynProvider` that owns the internal `Arc<dyn ErasedModelProvider>`. Lifetime-preserving boxed futures exist only at that erased seam. The network latency of provider calls dominates this allocation, while the non-generic agent type materially improves composition and testability. See [ADR 0002](../adr/0002-native-provider-trait-with-internal-erasure.md).

Cancellation should use explicit tokens/handles rather than relying on dropped futures. Tokio’s [`CancellationToken`](https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html) supports cloneable cancellation and child tokens, which maps well to parent/sub-agent runs; using it is a likely implementation choice, not yet a domain decision.

## Important divergences to make deliberately

- **Builder, not functional options.** Rust can validate configuration coherently at one boundary.
- **Enums/newtypes, not strings.** Status, roles, provider capabilities, termination reasons, IDs, and approval states should be typed.
- **Typed events, not interface downcasts.** Pattern matching should cover normal consumption.
- **Explicit run context, not ambient key/value context.** Cancellation and deadlines are separate from domain metadata.
- **Error taxonomy, not sentinel matching alone.** Preserve sources and retry advice.
- **Capability negotiation, not silent field dropping.** Provider differences should be inspectable and testable.
- **Decimal/fixed-point budgets, not `float64` enforcement.** Cost limits must not drift.
- **Traits at real seams only.** Do not copy every Go interface or create abstractions with one implementation.

## Open design tree

The interview should resolve these in dependency order:

1. Async runtime commitment: Tokio-only versus runtime-neutral.
2. Core public message and provider-capability model, including provider-native escape hatches.
3. Remaining tool details: metadata and error-code validation, schema-validation and strict-decoding implementation, serialization-failure behavior, retry-policy shape and backoff, trait bounds, generated names, and panic behavior. Function and stateful impl-block support, exactly one typed arguments struct, optional explicitly marked context, required explicit `name` and `description`, strict model-argument decoding, descriptions for every named model-visible property, open model-safe tool errors, explicit idempotency, conservative runtime-owned retries, deterministic duplicate-name rejection, structured JSON success outputs, and SDK-only retry metadata are accepted in [ADR 0003](../adr/0003-first-party-tool-macro-for-functions-and-impl-blocks.md).
4. Agent loop termination, retries, parallel tools, and side-effect safety.
5. Run/stream handle semantics, cancellation, backpressure, and replay.
6. Safety defaults for authorization and human approval.
7. Hooks versus middleware/layers.
8. Conversation, memory, and retrieval ownership boundaries.
9. Sub-agent topology and budget/cancellation/event propagation.
10. Initial provider set and conformance bar.
11. MCP, AG-UI, and A2A scope.
12. Durability scope and first backend.
13. Crate/workspace layout, feature flags, MSRV, semver, and release quality gates.

## Sources

1. Agentic Environment. [Agent SDK for Go repository, v0.3.6](https://github.com/agenticenv/agent-sdk-go/tree/d6a718fac0c6d116f7264717bb5158255ba6ff61). 14 September 2026.
2. Agentic Environment. [Architecture](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/architecture.mdx). v0.3.6.
3. Agentic Environment. [Runtime contracts](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/internal/runtime/runtime.go). v0.3.6.
4. Agentic Environment. [LLM interface](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/pkg/interfaces/llm.go). v0.3.6.
5. Agentic Environment. [Tool interface](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/pkg/interfaces/tool.go). v0.3.6.
6. Agentic Environment. [Feature documentation](https://github.com/agenticenv/agent-sdk-go/tree/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/features). v0.3.6.
7. Agentic Environment. [Durable execution](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/docs/advanced/durable-execution.mdx). v0.3.6.
8. Agentic Environment. [CI workflow](https://github.com/agenticenv/agent-sdk-go/blob/d6a718fac0c6d116f7264717bb5158255ba6ff61/.github/workflows/ci.yml). v0.3.6.
9. Rust Project. [Traits: dyn compatibility](https://doc.rust-lang.org/reference/items/traits.html#dyn-compatibility). Rust Reference.
10. Tokio contributors. [`CancellationToken`](https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html). Tokio Util documentation.
