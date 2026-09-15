# Slice 5: Temporal durable runtime

## Outcome

The same agent state machine proven locally can run as a durable Temporal workflow, survive worker failure and replay, Continue-As-New before history limits, and move all codec-capable workflow/activity payloads through the Slice 4 filesystem pipeline without exposing references to agent code.

The official Temporal Rust SDK reached 1.0 on September 4, 2026. The integration remains isolated in `agentive-temporal`, pins the tested 1.x compatibility range, and records any upstream limitation rather than weakening core semantics.

## Public seams

1. A Temporal runtime adapter starts, observes, cancels, and reconnects to an agent run using core run identities and outcomes.
2. Workflow and activity registration accepts an immutable agent/runtime definition and the Slice 4 `PayloadPipeline`.
3. Agent code, providers, and tools continue to receive canonical Rust values through their existing interfaces.

Temporal commands, activity payloads, history events, replay versions, task queues, data-converter mechanics, and SDK-Core details remain inside the adapter.

## Capability gate before full implementation

Before stabilizing public types, prove against Temporal Rust SDK 1.0 that the adapter can control payload conversion on clients and workers, execute/replay workflows deterministically, cancel activities, and Continue-As-New. If a required hook is missing, document the exact upstream gap and isolate the smallest temporary adapter rather than forking agent semantics.

## Test-first tracer order

1. A pure state-machine test drives one canonical model effect to a completed run identically under local and durable effect drivers.
2. A Temporal test workflow executes one scripted text response and returns the same result/history/usage as local execution.
3. A two-round tool workflow records effect identity so replay never re-executes a completed provider or tool effect.
4. Worker termination after scheduling, after completion-before-ack, and during retry recovers to one correct terminal result.
5. Temporal activity retry policy cannot multiply SDK provider/tool retries; ambiguous side effects remain non-retryable without idempotency.
6. Explicit cancellation propagates through workflow, provider activity, tool activity, and child delegation; late results cannot commit.
7. Workflow and pipeline fingerprints reject unexplained incompatible code or payload configuration.
8. Explicit version markers permit a tested compatible migration while unsupported history fails with an actionable typed error.
9. A deterministic threshold triggers Continue-As-New and carries complete canonical history, usage, budgets, identities, and pending state without semantic change.
10. Client, workflow, activity, and Codec Server all use the equivalent Slice 4 pipeline; a large text-and-image history is stored on the filesystem and arrives as typed canonical messages.
11. Missing or corrupt external payloads fail distinctly and follow Temporal-owned retry behavior only when their typed disposition permits it.
12. Reconnect by run identity reports current status, resumes supported event observation with opaque cursors, and returns the terminal result without starting duplicate work.
13. Parent/child agents use Temporal child workflows or the narrowest proven durable primitive while preserving Slice 3 identities, cancellation, events, and usage.

## Acceptance gates

- Pure local and Temporal state-machine conformance produce equivalent canonical outcomes for the same effect script.
- Crash/replay tests cover every effect boundary and demonstrate no unsafe duplicate externally visible action.
- A real ephemeral or local Temporal server test covers start, tool loop, cancellation, Continue-As-New, reconnect, and large-payload round trip.
- Workflow code performs no nondeterministic time, randomness, filesystem, provider, or tool operation outside explicit Temporal effects.
- All codec-capable Temporal payloads use External Storage; agent/activity interfaces never expose references.
- The tested Temporal Rust SDK version and stable-version compatibility policy are documented.
- All prior slice gates remain green.

## Explicit non-goals

- Restate, a custom durable journal, or a second workflow engine.
- Hiding unsupported upstream Temporal Rust behavior behind unverifiable claims.
- S3 storage, human approvals, MCP, or a separate conversation store.
