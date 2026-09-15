# Remaining design decisions

## Status

This is the complete decision checklist remaining after the accepted provider, tool, prompt-authority, usage, approval-deferral, and Temporal External Storage decisions. It deliberately excludes exact private type names and other reversible implementation details. Nothing in this document becomes accepted merely because it is recommended.

The fastest response format is: `Accept all recommendations except <numbers>`, followed by the preferred option or concern for each exception.

## A. First usable release and workspace

### 1. What is the first usable release?

- **A. Broad parity:** wait for providers, memory, retrieval, protocols, and durability.
- **B. Vertical core:** agent loop, tools and macro, events, scripted provider, and the first proven live integration; integrations follow in separate releases.
- **C. Core plus Temporal:** do not release until the vertical core and Temporal adapter both work.

### 2. Which crates exist initially?

- **A. One crate with feature flags.**
- **B. Create the complete future workspace immediately.**
- **C. Start with `agentive`, `agentive-macros`, `agentive-test`, and the first working provider crates; add integration crates only when implementation begins.**

### 3. Which async runtime does core support?

- **A. Tokio only.**
- **B. Runtime-neutral core with adapters for every executor.**
- **C. Runtime-neutral public traits but Tokio-owned execution internals.**

### 4. What is the compiler-support policy?

- **A. Latest stable Rust only.**
- **B. Declare one concrete MSRV no newer than roughly six months at release time; test MSRV and current stable; raise MSRV only in a documented minor release.**
- **C. Freeze the initial MSRV indefinitely.**

Rust itself has no LTS channel. Tokio dependencies should use a supported Tokio LTS minor where the selected dependency graph permits it.

### 5. How are optional integrations packaged?

- **A. Large feature matrix on `agentive`.**
- **B. Separate crates for providers, protocols, stores, telemetry, and durable runtimes; core features remain small and additive.**
- **C. Put every integration in downstream repositories.**

### 6. Which providers establish the abstraction?

- **A. Scripted provider only.**
- **B. Scripted provider plus OpenAI and Anthropic wire adapters.**
- **C. Scripted provider plus a Codex App Server conformance spike using ChatGPT subscription authentication. Ship it as `CodexProvider` only if it obeys the raw provider contract; otherwise expose an honest `CodexRuntime` integration. Defer OpenAI and Anthropic wire adapters.**

## B. Provider protocol

### 7. What happens when a requested capability is unsupported?

- **A. Silently omit it.**
- **B. Typed capability negotiation and a request-compilation error unless the caller explicitly selects a documented fallback.**
- **C. Send it and let the provider decide.**

### 8. How is structured output represented?

- **A. JSON mode only.**
- **B. Typed support levels (`None`, `Json`, `JsonSchema`) plus a typed `run_structured<T>` path; exact-schema requests fail before transport when unsupported.**
- **C. Provider-specific untyped request JSON.**

### 9. How are provider-specific controls exposed?

- **A. An untyped JSON map on every model request.**
- **B. No escape hatch at all.**
- **C. Typed provider-builder configuration and narrowly scoped typed extensions; canonical orchestration never depends on them.**

### 10. Who owns provider retries?

- **A. Each provider adapter retries internally.**
- **B. The agent runtime applies one policy to typed provider errors and provider retry hints; adapters make one transport attempt.**
- **C. No provider retries anywhere.**

### 11. What happens when native streaming is unavailable or fails?

- **A. `stream()` is unsupported.**
- **B. The agent stream emits a synthetic final text event for non-streaming providers; a failed native stream may retry only before any user-visible delta was emitted.**
- **C. Retry and replay the entire stream even after partial output.**

## C. Messages, images, and context

### 12. What are the canonical conversation roles?

- **A. Arbitrary role strings.**
- **B. `User`, `Assistant`, and correlated `Tool` items; trusted SDK/agent/run instructions remain outside conversation roles.**
- **C. Copy every provider's role vocabulary into core.**

### 13. Which image sources are canonical in v1?

- **A. Inline bytes only.**
- **B. URLs only.**
- **C. Typed inline bytes with validated media type plus validated URLs; Temporal offloading remains transparent.**

### 14. Are generic files a v1 canonical content part?

- **A. Yes, alongside text and images.**
- **B. No; defer files, audio, and video. Tools may return structured resource references.**

### 15. What happens when complete context does not fit?

- **A. Keep the last N messages.**
- **B. Automatically summarize or compact.**
- **C. V1 `AllOrError`: preserve the requested conversation and return a typed preflight context-limit error instead of silently discarding content.**

### 16. Where do context limits and token estimates come from?

- **A. Hard-code one global limit.**
- **B. Built-in provider/model knowledge plus an explicit typed override; custom unknown models require a configured limit or explicit unsafe opt-out. Estimates are labeled exact, estimated, or unknown and reserve output capacity.**
- **C. Rely only on provider rejection.**

### 17. What is the conversation-store consistency model?

- **A. Load and overwrite an entire message array.**
- **B. Append immutable complete turns using a revision/compare-and-append contract; concurrent conflicts are explicit. Preserve full history and let context policy decide what is sent.**
- **C. Last writer wins.**

### 18. Which runtime-generated items become model-visible?

- **A. Every event and diagnostic.**
- **B. Only canonical protocol items needed for the next model decision, such as correlated tool results; retries, timings, limits, and diagnostics remain SDK-only.**
- **C. None, including tool results.**

### 19. How stable is the XML fallback format?

- **A. Ad hoc strings per provider.**
- **B. One escaped deterministic versioned rendering owned by core, snapshot-tested but not exposed as the canonical storage model.**
- **C. Public XML types become the canonical request model.**

## D. Tools and the agent loop

### 20. Do we accept the hardened `ToolInvocation`/`ToolContext` design?

- **A. Yes: stable serializable invocation identity separated from borrowed ephemeral cancellation/deadline controls, with no arbitrary extension bag.**
- **B. Replace it with a string-keyed context map.**
- **C. Give tools the entire mutable run state.**

### 21. How are names and error codes validated?

- **A. Accept arbitrary strings and fail at providers.**
- **B. Validated newtypes using a documented portable ASCII subset; compile-time validation for macro literals and runtime parsing for configuration.**
- **C. Separate naming rules for every provider in application code.**

### 22. How is strict nested argument decoding enforced?

- **A. Trust `serde` attributes alone.**
- **B. Validate model JSON against the exact completed JSON Schema before deserializing, then deserialize into the typed arguments structure.**
- **C. Deserialize into `Value` and let each tool inspect it.**

### 23. What happens on output serialization failure or tool panic?

- **A. Convert either into model-visible debug text.**
- **B. Treat serialization failure as an internal tool-protocol error; isolate local tool execution so an unwind becomes a private terminal run error when the panic strategy permits it. Never retry or expose panic content.**
- **C. Let either crash the whole process unconditionally.**

### 24. What is the concrete retry policy?

- **A. Retry every failure three times.**
- **B. `RetryPolicy::none()` by default for tools; explicit bounded exponential backoff with full jitter only when tool idempotency, error disposition, remaining attempts, and run state all permit it.**
- **C. Tool implementations sleep and retry themselves.**

### 25. How are multiple tool calls executed?

- **A. Always concurrently.**
- **B. Sequential by default; explicit parallel-safe semantics permit bounded concurrency, while final tool-result messages retain provider call order.**
- **C. Registration order decides implicitly.**

### 26. How are malformed or unknown model tool calls handled?

- **A. Crash the run immediately.**
- **B. Append a safe correlated `invalid_arguments` or `tool_not_found` result so the model may repair its request within the normal iteration bound; malformed provider protocol that cannot be correlated fails the run.**
- **C. Guess the intended tool or arguments.**

### 27. What happens at the agent-loop limit?

- **A. Make a hidden extra model call with tools removed.**
- **B. Return a typed incomplete termination with accumulated usage and execution records; no uncounted final call. Default to a finite model-call limit, configurable per agent/run.**
- **C. Run indefinitely unless cancelled.**

## E. Runs, events, limits, and middleware

### 28. What is the run interface?

- **A. Only `run(...).await`.**
- **B. `start(...) -> RunHandle` is the control primitive; `run(...).await` is ergonomic sugar. Dropping a handle or waiter never cancels work; cancellation is explicit.**
- **C. Dropping the future cancels the run.**

### 29. What are event-stream delivery semantics?

- **A. Unbounded channels.**
- **B. A typed non-exhaustive `Stream` with validated ordering and bounded backpressure while attached; dropping observation does not cancel the run. Replay uses opaque runtime-owned cursors only where supported.**
- **C. Best-effort events that may silently drop tool and terminal events.**

### 30. Which budgets ship initially?

- **A. Model-call, elapsed-time, and token limits; defer money until pricing and fixed-point semantics are designed.**
- **B. Floating-point USD limits immediately.**
- **C. No limits.**

### 31. Do mutable hooks/middleware ship in v1?

- **A. A separate before/after hook trait for every operation.**
- **B. Defer input-mutating middleware until concrete guardrail use cases exist; ship typed events and `tracing` instrumentation first.**
- **C. One untyped global callback.**

## F. Workflow capabilities and protocols

### 32. When do sub-agents ship, and how do they compose?

- **A. In the first core vertical slice.**
- **B. Immediately after the core loop stabilizes: delegation is an explicit tool, providers may differ, build-time cycles are rejected where detectable, depth is bounded, and usage/cancellation aggregate through the run tree.**
- **C. Defer indefinitely.**

### 33. Which stateful knowledge features ship first?

- **A. Conversation first; expose retrieval through ordinary tools initially; defer dedicated memory extraction and vector-store traits until real adapters exist.**
- **B. Conversation, memory, and retrieval together in core.**
- **C. No separate stateful store in v1. Runs use complete canonical message histories, and a long-lived Temporal workflow may hold that history while External Storage transparently offloads its payloads. Add cross-run conversation storage only when a real use case requires it.**

### 34. Which external protocols come first?

- **A. MCP after tools/sub-agents, then an AG-UI adapter after event semantics stabilize; defer A2A until there is a concrete interoperability requirement.**
- **B. MCP, AG-UI, and A2A in v1 core.**
- **C. No protocol integrations in v1; native registered tools are sufficient. Add a protocol only for a concrete interoperability requirement.**

## G. Durability and Temporal payload infrastructure

### 35. What is the durability architecture and first backend?

- **A. Write a local-only loop and retrofit durability later.**
- **B. Model the loop internally as a serializable deterministic state machine with explicit effects from the start; execute locally first, then implement Temporal as the first durable adapter. Defer Restate and a local journal.**
- **C. Put Temporal concepts directly into core.**

### 36. How does Temporal handle upgrades and long histories?

- **A. Hope workers remain compatible.**
- **B. Persist a stable workflow/pipeline fingerprint, reject unexplained mismatches, support explicit version migrations, and use deterministic Continue-As-New thresholds before history limits.**
- **C. Restart workflows after every deployment.**

### 37. What is the storage/codec error contract?

- **A. One string error.**
- **B. Typed kinds with retry advice: transient I/O, throttling, and temporary unavailability may be retryable; missing objects, checksum mismatch, malformed references, unknown IDs, unsupported versions, and size-limit violations are terminal. The owning runtime performs retries.**
- **C. Each Storage Driver retries internally.**

### 38. What is the External Storage reference wire format?

- **A. Rust `bincode` tied to struct layout.**
- **B. A small versioned canonical JSON envelope inside a Temporal Payload with a stable encoding marker; validated fields include reference version, `StorageId`, `ObjectId`, encoded size, checksum algorithm/value, codec stack, and pipeline identity.**
- **C. An opaque provider-specific URL.**

### 39. What interface does the Codec Server expose, and who secures it?

- **A. Ship a standalone unauthenticated executable.**
- **B. Ship a framework-neutral HTTP handler/Tower service in the Temporal crate plus an Axum convenience adapter. The application mounts it behind its authentication, authorization, request limits, and TLS; the SDK never binds a socket.**
- **C. Axum-only executable and configuration format.**

### 40. How is payload-pipeline configuration kept consistent?

- **A. Independent client, worker, and Codec Server configuration with no verification.**
- **B. One immutable `PayloadPipeline` builder computes a stable typed fingerprint recorded in references; each process constructs an equivalent pipeline, and servers may retain explicitly registered historical decoders. Mismatch is explicit.**
- **C. Store credentials and complete configuration inside each Temporal reference.**

### 41. How are decompression bombs and oversized reads prevented?

- **A. Trust the reference size.**
- **B. Enforce configurable typed limits before download, during streaming download, and during decompression. Start with 64 MiB per decoded payload and a separately configurable batch limit.**
- **C. Unlimited payloads because storage is external.**

### 42. What proves the payload infrastructure works?

- **A. Unit-test concrete implementations independently.**
- **B. Reusable conformance suites for every Storage Driver and Payload Codec; in-memory fake, temporary-filesystem tests, corruption and retry fault injection, Codec Server HTTP tests, and an end-to-end Temporal round trip. Object-store testing is added with the first object-store driver.**
- **C. Only a live AWS test.**

## H. Release gates

### 43. Which checks are release-blocking?

- **A. Formatting and unit tests only.**
- **B. Protected-branch CI runs formatting, Clippy with warnings denied, nextest plus doctests, MSRV check, feature-power-set checks, docs with warnings denied, dependency/license/advisory policy, public API semver checks after first release, platform checks, and conformance suites. Coverage ratchets but is not a substitute for behavioral tests.**
- **C. Mandatory Git hooks are the source of truth.**

## Consolidated recommendations

| # | Recommendation | Why |
|---:|:---:|---|
| 1 | B | Ship a useful vertical core without waiting for every integration. |
| 2 | C | Create crates only when they contain working code. |
| 3 | A | Tokio gives the cleanest ecosystem fit and cancellation/timer semantics. |
| 4 | B | A concrete tested MSRV balances adoption and dependency health. |
| 5 | B | Integration crates keep core lean and dependency-safe. |
| 6 | C | The scripted fake proves determinism; the Codex spike validates whether subscription-backed Codex actually fits the provider seam without distorting it. |
| 7 | B | Unsupported behavior must be explicit, never silently discarded. |
| 8 | B | Typed support levels preserve portability without weakening schemas. |
| 9 | C | Typed adapter configuration permits escape hatches without infecting core. |
| 10 | B | One runtime policy prevents nested retries and inconsistent semantics. |
| 11 | B | Streaming remains ergonomic without duplicating already-emitted output. |
| 12 | B | Trusted instructions are not conversation messages. |
| 13 | C | Bytes and URLs cover practical image input while remaining explicit. |
| 14 | B | Generic files add provider and security complexity not required for v1. |
| 15 | C | Preserve meaning; never silently amputate history. |
| 16 | B | Known limits are enforced and uncertainty stays visible. |
| 17 | B | Append-only turns prevent lost concurrent updates and preserve history. |
| 18 | B | Models see protocol facts, while operators retain diagnostics. |
| 19 | B | Deterministic fallback without making XML the domain model. |
| 20 | A | The hardened context is small, explicit, durable-friendly, and testable. |
| 21 | B | Portable validated identities prevent provider-time surprises. |
| 22 | B | Schema validation is the only reliable way to enforce nested strictness. |
| 23 | B | Developer bugs remain private and cannot take down normal local execution. |
| 24 | B | Side effects are never retried merely because transport failed. |
| 25 | B | Safety and determinism by default; concurrency must be declared. |
| 26 | B | Repairable model mistakes should not become infrastructure failures. |
| 27 | B | Limits remain honest and usage remains complete. |
| 28 | B | Simple calls stay simple while control and durable runs remain possible. |
| 29 | B | Typed ordered events with explicit backpressure are testable and portable. |
| 30 | A | Enforce what can be measured reliably; defer pricing policy. |
| 31 | B | Events and tracing cover v1 without freezing speculative mutation hooks. |
| 32 | B | Stabilize delegation on top of the proven loop rather than inside it. |
| 33 | C | Workflow state plus transparent External Storage covers the current durable-conversation need without introducing a second persistence abstraction. |
| 34 | C | Native tools cover current use cases; protocol integrations would be speculative scope. |
| 35 | B | The local executor stays simple without making Temporal a retrofit. |
| 36 | B | Durable histories require explicit compatibility and bounded growth. |
| 37 | B | Typed advice plus runtime-owned retries matches the accepted error model. |
| 38 | B | Versioned JSON is inspectable, portable, and already supported by the stack. |
| 39 | B | Applications own network security and deployment; the SDK owns protocol behavior. |
| 40 | B | Fingerprints detect drift without putting secrets in history. |
| 41 | B | External storage does not make unbounded allocation safe. |
| 42 | B | The same contract must pass against fakes, real adapters, corruption, and Temporal. |
| 43 | B | CI is enforceable; Git hooks are optional convenience only. |

## Recommended implementation sequence after acceptance

1. Scaffold the minimal workspace and CI/MSRV gates.
2. Implement canonical messages, provider protocol, scripted provider, and conformance suite.
3. Implement typed/manual tools and the `#[tool]` macro with compile-fail tests.
4. Implement the deterministic loop, run handles, cancellation, usage, and events against the scripted provider.
5. Run the Codex App Server conformance spike using ChatGPT subscription authentication; ship it as a provider only if it satisfies the provider contract, otherwise as a separate runtime integration.
6. Add sub-agent delegation.
7. Implement the runtime-neutral durable state/effect representation and Temporal adapter, keeping complete conversation messages in workflow state.
8. Add External Storage, the filesystem driver, codec chain, Codec Server library, and their conformance/fault suites.
9. Stabilize typed events and core `tracing`. Defer OpenAI, Anthropic, MCP, conversation storage, observability exporters, S3, and other integrations until a real use case asks for each one.
