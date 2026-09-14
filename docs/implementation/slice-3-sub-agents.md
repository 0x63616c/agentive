# Slice 3: explicit sub-agent delegation

## Outcome

An agent can delegate one task to another SDK agent through an ordinary, explicit tool definition while the parent retains bounded control, typed events, cancellation, committed history, and aggregate usage.

## Public seams

1. Agent construction registers a named sub-agent as a delegation tool.
2. Parent `Agent::run` and `RunHandle` expose the complete parent/child behavior.
3. Existing canonical tool, event, history, usage, error, and provider types remain the only protocol.

There is no separate orchestration DSL, hidden recursive run entry point, or sub-agent-specific provider protocol.

## Test-first tracer order

1. A parent using `ScriptedProvider` delegates through one registered child tool and returns the child's structured result.
2. Parent and child use different scripted providers without changing either agent interface.
3. Delegation produces stable parent/child run identities and attributed ordered events.
4. Child model and tool usage aggregates exactly once into the parent result while remaining inspectable per run.
5. Parent cancellation propagates to an active child and produces one terminal outcome at each level.
6. Child failure becomes the agreed safe delegation-tool error; private child diagnostics remain in execution records.
7. Agent construction rejects statically detectable cycles and delegation-tool name collisions.
8. Runtime depth limits reject dynamically reached excess depth without starting another child.
9. Parent model-call, elapsed-time, and token budgets bound the complete run tree; a child cannot escape remaining limits.
10. Parallel-safe delegation obeys the existing bounded tool concurrency and provider-order result rules.
11. Committed parent history contains one correlated delegation tool result, never the child's internal conversation or failed attempts.

## Acceptance gates

- Delegation is implemented entirely through the existing tool execution seam.
- No new message roles, retry engine, event transport, budget representation, or provider trait is introduced.
- Cycle, depth, cancellation, usage, error disclosure, event ordering, and history tests pass through public agent interfaces.
- All Slice 1 gates remain green.

## Explicit non-goals

- Remote agents, A2A, MCP, human approvals, delegation discovery, or dynamic network registration.
- Persisting child conversation history as a separate conversation.
- A graph-building DSL.
