# Recommended Rust foundation stack

## Status

Research and recommendations for the design interview. The specification and ADRs are authoritative for accepted decisions; this document links evidence and labels recommendations that remain open.

## Provider abstraction — accepted

The best fit is a two-layer design:

1. SDK users implement a native, statically dispatched `ModelProvider` trait whose methods return `impl Future + Send`.
2. `AgentBuilder::provider` converts that implementation into an internal erased provider stored by `Agent`.

This keeps provider implementations pleasant, avoids exposing boxed futures in the normal authoring API, and still gives the agent a single non-generic type capable of holding OpenAI, Anthropic, a scripted mock, or a fallback router. Rust supports return-position `impl Trait` in traits, but those traits are not dyn-compatible; the internal erased trait is the narrow bridge across that language boundary. See the [Rust Reference on return-position `impl Trait`](https://doc.rust-lang.org/reference/types/impl-trait.html#return-position-impl-trait-in-traits-and-trait-implementations) and [dyn compatibility](https://doc.rust-lang.org/reference/items/traits.html#dyn-compatibility).

The provider remains stable for an agent's lifetime. Routing, fallback, load balancing, and model selection are provider implementations themselves. Different agents and sub-agents may use different providers. This avoids per-run mutation of validated agent configuration while retaining every useful form of swapping.

Illustrative shape:

```rust
pub trait ModelProvider: Send + Sync + 'static {
    fn generate(
        &self,
        request: ModelRequest,
    ) -> impl Future<Output = Result<ModelResponse, ProviderError>> + Send + '_;

    fn stream(
        &self,
        request: ModelRequest,
    ) -> impl Future<Output = Result<ModelStream, ProviderError>> + Send + '_;

    fn capabilities(&self) -> ProviderCapabilities;
}

impl AgentBuilder {
    pub fn provider<P>(self, provider: P) -> Self
    where
        P: ModelProvider,
    {
        self.provider = Some(DynProvider::new(provider));
        self
    }
}

pub(crate) struct Agent {
    provider: DynProvider,
}
```

`DynProvider` owns an `Arc<dyn ErasedModelProvider>`. Its erased methods return `BoxFuture<'_, ...>`, preserving any borrow of the provider rather than incorrectly requiring a `'static` future. The allocation happens only at the already-dynamic orchestration seam and is negligible next to network model calls. The accepted decision is recorded in [ADR 0002](../adr/0002-native-provider-trait-with-internal-erasure.md).

## Core runtime and protocol dependencies

Use workspace dependencies so versions and features are controlled once.

| Crate | Role | Policy |
|---|---|---|
| `tokio` 1.x | Async task runtime, synchronization, clocks, timers | Standard runtime; minimal production features, fuller dev features |
| `tokio-util` 0.7 | Hierarchical cancellation tokens | Wrap behind SDK run context and handles rather than exposing everywhere |
| `futures-core` 0.3 | Public `Stream` vocabulary | Public contract only where streaming is required |
| `futures-util` 0.3 | Stream combinators and boxed adapters | Implementation dependency, not prelude surface |
| `serde` 1.x | Wire and durable-state serialization | Derive enabled |
| `serde_json` 1.x | Provider/tool JSON boundary | Preserve raw JSON only at real protocol escape hatches |
| `thiserror` 2.x | Typed library error implementations | No `anyhow` in library APIs |
| `tracing` 0.1 | Structured instrumentation | Core emits spans/events; exporters live elsewhere |
| `uuid` 1.x | Run, message, and tool-call IDs | UUIDv7 where locally generated ordering is useful; serde enabled |
| `schemars` 1.x | Rust type to JSON Schema for tools and structured outputs | Typed authoring path; raw schema remains possible |

Tokio is the pragmatic runtime choice. It is mature, ubiquitous across HTTP and protocol clients, and currently publishes LTS minors; its documentation recommends fixed LTS minors when consumers need that stability. The workspace should specify compatible 1.x requirements rather than freeze transitive users to one patch. See [Tokio’s release and MSRV policy](https://docs.rs/crate/tokio/latest).

## Provider adapter dependencies

| Crate | Role | Policy |
|---|---|---|
| `reqwest` 0.13 | HTTP, streaming bodies, proxy/TLS integration | Provider crates only; default features off, Rustls selected |
| `secrecy` 0.10 | Prevent accidental secret exposure through debug/display | Provider configuration only |
| `url` 2.x | Validated base URLs | Provider/protocol crates only |

Do not standardize on provider-specific client crates until each adapter is designed. Some vendors have mature official Rust SDKs and some do not; every adapter must satisfy the same wire and conformance tests either way.

Do not put HTTP, TLS, or provider credentials in the core crate.

## Test dependencies and tools

| Crate/tool | Role |
|---|---|
| `proptest` 1.x | State-machine, serialization, event-ordering, and malformed-input properties |
| `trybuild` 1.x | Compile-pass/fail tests for public traits, builders, and macros |
| `wiremock` 0.6 | Provider HTTP/SSE adapter tests without real credentials |
| `insta` 1.x | Reviewed wire fixtures and event-sequence snapshots, used selectively |
| `tempfile` 3.x | Isolated persistence and recovery tests |
| `cargo-nextest` | Fast, reliable test execution and retries for genuinely flaky external tests only |
| `cargo-llvm-cov` | Coverage reporting |
| `cargo-semver-checks` | Public API compatibility gate |
| `cargo-deny` | License, advisory, duplicate, and source policy |
| `rustfmt` and Clippy | Formatting and lint gates with warnings denied in project code |

The primary mock remains a handwritten `ScriptedProvider`, not a mocking framework. It models ordered model interactions more clearly than generated method expectations and becomes executable documentation for workflows.

## Dependencies to avoid in the core baseline

- `async-trait`: unnecessary if native traits are erased internally; revisit only if compiler ergonomics prove materially worse.
- `anyhow`: excellent for CLI/examples, inappropriate as the public library error contract.
- `tower`: potentially useful for provider transport middleware, but premature for the agent loop until retry and hook semantics are settled.
- `chrono`: use `std::time` and wire timestamps unless calendar arithmetic appears.
- `rust_decimal`: defer until monetary budget semantics are accepted; integer micros may be sufficient.
- `mockall`: the scripted test provider and small hand-written fakes better express ordered agent behavior.

## Feature policy

- Core defaults should be useful but lean.
- Provider, storage, protocol, telemetry-export, and durable-runtime integrations belong in separate crates rather than a giant feature matrix on one crate.
- Features should be additive; CI must test each supported feature combination and a representative all-features build.
- Public APIs should expose ecosystem-standard traits (`Future`, `Stream`, serde types where appropriate) rather than concrete channels or executor internals.

## Gate enforcement

Git hooks are optional convenience, not enforcement: they can be skipped, are awkward to distribute, and make slow checks disruptive. The required source of truth should be protected-branch CI. Local development should invoke the same commands through one small repository command; add an `xtask` only if the command graph later becomes complicated.

Recommended pull-request checks:

1. `cargo fmt --all --check`.
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
3. `cargo nextest run --workspace --all-features --profile ci`, plus `cargo test --doc --workspace` because nextest does not replace doctests.
4. `cargo check` on the declared MSRV toolchain.
5. `cargo hack check --feature-powerset` once optional features exist.
6. `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`.
7. `cargo deny check` for advisories, licenses, banned/duplicate dependencies, and sources.
8. `cargo semver-checks` against the latest released version after the first release.
9. `cargo llvm-cov` with a ratcheted coverage floor, while keeping behavioral conformance—not percentage—the acceptance criterion.
10. Linux, macOS, and Windows checks for the portable core; platform-specific integrations may use narrower matrices.

Recommended scheduled checks:

- Latest compatible dependency resolution and direct-minimum dependency resolution.
- Fuzz targets for event, provider, tool, and durable-state decoders.
- Live provider smoke tests with tightly scoped secrets and spend limits.
- Durable-runtime crash and replay suites when those runtimes exist.

Recommended repository rules:

- Require all PR checks through branch protection.
- Require review for public API or security-sensitive changes.
- Forbid unsafe code in core unless a future ADR approves a narrowly reviewed exception.
- Commit `Cargo.lock` for the workspace's applications, examples, and CI reproducibility while publishing library crates with normal compatible dependency requirements.
- Do not automatically retry deterministic unit tests; nextest retries are reserved for explicitly classified external tests and still report flakiness.

## Macro policy — accepted direction

**Accepted direction:** ship a first-party procedural macro for authoring tools from the first release. It supports both stateless async free functions and stateful impl blocks. The underlying plain Rust trait remains public and usable directly, both as an escape hatch and as the contract the generated code implements.

The non-macro path should remain excellent:

```rust
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct WeatherArgs {
    /// City name, including a region or country when ambiguous.
    city: String,
}

impl Tool for Weather {
    type Args = WeatherArgs;
    type Output = Weather;

    fn call<'a>(
        &'a self,
        context: &'a ToolContext,
        args: WeatherArgs,
    ) -> impl Future<Output = Result<Weather, ToolError>> + Send + 'a {
        async move { /* ... */ }
    }
}
```

For stateless tools, the proc-macro crate provides an attribute such as:

```rust
#[agents::tool(
    name = "weather",
    description = "Fetch the current weather for a city."
)]
async fn weather(
    #[tool(context)] context: &ToolContext,
    args: WeatherArgs,
) -> Result<Weather, ToolError> {
    // ...
}
```

Every macro-authored tool accepts exactly one typed arguments struct. It may additionally accept an explicitly marked `#[tool(context)] context: &ToolContext` before that struct; context is SDK-injected and excluded from JSON Schema. A no-argument tool uses an empty struct. Provider-facing `name` and `description` string literals are required and are never inferred from Rust identifiers or doc comments. Dynamic decoding rejects unknown fields at any depth before executing the tool, and the generated schema describes the same closed shape; intentional extensibility must be modeled explicitly in the arguments type. Every named model-visible property, including nested properties, has a non-empty description, normally authored as a Rust field doc comment and projected into the schema by `schemars`; examples and constraints remain optional. The completed schema is validated before a run. The macro may generate only the mechanical adapter: metadata, schema lookup, JSON argument decoding, output serialization, and the dynamic `Tool` implementation. It must not hide control flow, retries, approvals, provider calls, or agent construction. `cargo expand` output and `trybuild` diagnostics are required tests.

Tool functions may return `Result<T, E>` only when `E: Into<ToolError>`, with `ToolResult<T>` as the convenient direct alias. `ToolError` requires an open, stable application-defined code, holds an explicit model-safe message, records whether the failure is terminal or retryable, and may retain an internal source. The SDK never derives model-visible content from an arbitrary error's `Display` or debug representation; domain errors cross that boundary through an explicit conversion.

The runtime owns retries. A tool opts into repeat safety explicitly through idempotent semantics, while execution policy owns attempts and backoff. A retry happens only for an idempotent tool returning a retryable error when the policy has capacity and the run remains active. Defaults are no automatic repeat safety and one attempt. The same logical invocation ID and idempotency key survive across attempts.

Tool names are unique within an agent's effective registry. Agent construction rejects static collisions, while dynamic registration rejects the new entry atomically and preserves the existing tool. Registration order never chooses a winner silently.

Successful tool outputs implement `serde::Serialize` and are erased to canonical `serde_json::Value`; only provider adapters perform wire rendering. The runtime never falls back to `Display` or debug formatting. Binary and streaming tool results are deferred from the first release in favor of structured resource references.

One logical tool call produces one final model-visible success or safe error. Attempts, retry delays, timing, SDK identity, idempotency keys, and diagnostics remain in `ToolContext`, execution records, events, and tracing as appropriate; intermediate failures never enter conversation history.

For tools that own an HTTP client, database pool, or application service, the same macro applies to an impl block containing the call method. This avoids forcing real stateful tools through the manual trait path.

The accepted function/impl-block, single-arguments-struct, explicit-metadata, strict-decoding, required field-guidance, open safe-error, conservative runtime-owned retry, duplicate-name, structured-output, and SDK-only retry-metadata decisions are recorded in [ADR 0003](../adr/0003-first-party-tool-macro-for-functions-and-impl-blocks.md). Metadata and error-code validation, schema-validation and strict-decoding implementation, serialization-failure behavior, retry-policy shape and backoff, trait bounds, generated names, and panic behavior remain under interview.

## Current published versions observed during research

As of 14 September 2026: Tokio 1.53.1, tokio-util 0.7.19, futures 0.3.34, serde 1.0.229, serde_json 1.0.151, thiserror 2.0.20, tracing 0.1.44, uuid 1.26.1, schemars 1.2.2, reqwest 0.13.5, secrecy 0.10.3, proptest 1.11.0, trybuild 1.0.121, wiremock 0.6.5, insta 1.48.0, and tempfile 3.27.0. These are research observations, not exact-version pins; workspace requirements should follow Cargo semver conventions and the eventual MSRV policy.
