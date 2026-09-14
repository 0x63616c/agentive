# Tool context design

## Status

Hardened proposal after review. Not accepted yet.

## Objective

Give a tool the minimum runtime information needed to cooperate safely with one invocation without turning context into a global key/value bag, dependency container, or hidden workflow interface.

## Separate durable identity from live control

`ToolInvocation` contains stable, serializable facts. A durable runtime persists it and reconstructs the same identity during retry or recovery.

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ToolInvocation {
    run_id: RunId,
    invocation_id: ToolInvocationId,
    provider_call_id: Option<ProviderToolCallId>,
    tool_name: ToolName,
    model_round: ModelRound,
    idempotency_key: IdempotencyKey,
}
```

The SDK-generated `invocation_id` is authoritative. A provider call ID is optional transport correlation data because not every provider supplies one.

`ToolContext` contains ephemeral controls for one execution attempt. It is passed by reference, cannot be serialized, is not cloneable, and has no public production constructor.

```rust
pub struct ToolContext {
    invocation: ToolInvocation,
    attempt: AttemptNumber,
    control: InvocationControl,
}

impl ToolContext {
    pub fn invocation(&self) -> &ToolInvocation;
    pub fn attempt(&self) -> AttemptNumber;
    pub fn remaining(&self) -> Option<Duration>;
    pub fn is_cancelled(&self) -> bool;
    pub fn cancellation_reason(&self) -> Option<CancellationReason>;
    pub async fn cancelled(&self) -> CancellationReason;
}
```

Newtypes prevent accidental mixing of run IDs, provider call IDs, tool names, invocation IDs, and idempotency keys. Accessors expose values without allowing mutation.

## Macro injection

The context is optional and is never included in the model-visible JSON Schema. An explicit parameter marker makes macro parsing robust to local variable names, imports, and type aliases.

```rust
/// Search indexed documents.
#[tool(
    name = "search",
    description = "Search indexed documents for relevant passages."
)]
async fn search(
    #[tool(context)] context: &ToolContext,
    args: SearchArgs,
) -> Result<SearchOutput, SearchError> {
    tokio::select! {
        result = perform_search(args) => result,
        _ = context.cancelled() => Err(SearchError::Cancelled),
    }
}
```

A tool that does not need runtime controls omits the context parameter.

```rust
#[tool(
    name = "add",
    description = "Add two integers and return their sum."
)]
async fn add(args: AddArgs) -> Result<AddOutput, ToolError> {
    Ok(AddOutput { sum: args.left + args.right })
}
```

## Idempotency

`invocation_id` and `idempotency_key` identify the logical invocation, not an individual attempt. They remain unchanged across retries, worker restarts, and durable replay. `attempt` starts at one and increases only when the SDK actually calls the tool again.

```rust
#[tool(
    name = "charge_card",
    description = "Charge a payment card and return a receipt."
)]
async fn charge_card(
    #[tool(context)] context: &ToolContext,
    args: ChargeArgs,
) -> Result<Receipt, PaymentError> {
    payment_client
        .charge(args.amount)
        .idempotency_key(context.invocation().idempotency_key())
        .send()
        .await
}
```

An idempotency key does not make an operation idempotent by itself. The downstream system must honor it. Retry permission belongs to the tool execution policy, not to `ToolContext`.

## Cancellation, deadlines, and terminal arbitration

- Cancellation is cooperative; Rust cannot forcibly stop arbitrary synchronous or external work safely.
- The runtime checks cancellation before starting each attempt.
- Completion, failure, cancellation, and timeout compete through one runtime-owned terminal-state commit gate. Exactly one transition from running to a terminal state succeeds.
- If cancellation or timeout commits first, a late tool result is discarded and cannot advance the agent loop. If completion commits first, later cancellation does not rewrite history.
- The execution policy stores a timeout duration. A durable attempt additionally records the runtime's durable start time or deadline so downtime counts toward the timeout.
- On recovery, an expired attempt times out without invoking the tool. Otherwise the runtime reconstructs a fresh monotonic process-local deadline from the durable remaining time.
- A tool may inspect remaining time, but the runtime still enforces its own timeout around the call.
- Cancellation reason is typed: user request, parent cancellation, deadline exceeded, or runtime shutdown. Internal failures remain errors, not cancellations.
- Dropping a caller's wait future does not cancel the invocation. Cancellation occurs only through the run control interface or an owning runtime shutdown policy.

## Explicit exclusions

`ToolContext` does not contain:

- Tool dependencies such as clients, pools, or application services. Those belong on a stateful tool struct.
- Model-generated arguments. Those belong in the typed arguments struct.
- Provider access or a method to call the model recursively.
- Approval or authorization powers.
- A tracing span or logger handle; normal `tracing` instrumentation propagates outside this interface.
- Arbitrary `Any`, JSON, or string-keyed extensions.
- A task-spawn method that could create work untracked by the run.

Per-run application identity and authorization claims are a separate unresolved design problem. They must not be smuggled into this interface without deciding serialization, secrecy, and durable-runtime semantics.

## Testing interface

The public production type has no public constructor. The test-support crate provides a builder that creates valid contexts without exposing runtime internals.

```rust
let context = TestToolContext::builder()
    .run_id("run-123")
    .invocation_id("invocation-1")
    .provider_call_id("provider-call-1")
    .tool_name("search")
    .model_round(1)
    .idempotency_key("run-123/invocation-1")
    .attempt(1)
    .build();

let output = search(&context, SearchArgs { query: "rust".into() }).await?;
```

Tests cover stable identity across attempts, optional provider call IDs, unique identity across logical invocations, parent cancellation, deadline and completion races in both directions, downtime expiry, late-result discard, cancellation reasons, direct function testing, and macro exclusion of context from JSON Schema.

## Interface depth

The interface exposes identity, cancellation, and timing only. The implementation hides token propagation, cancellation races, durable identity reconstruction, timeout enforcement, and late-result rejection. Deleting this module would force those rules into every tool adapter and runtime, so the seam earns its place.
