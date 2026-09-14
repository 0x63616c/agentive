# Slice 2: Codex subscription integration

## Outcome

A Rust application can execute the SDK's agreed text, image, and tool workflow using the user's existing locally authenticated Codex subscription. The slice ends with a working integration and an ADR that names the integration according to the behavior actually proven.

## Decision gate before public stabilization

Codex App Server exposes threads, turns, events, approval requests, and experimental dynamic tools. That may be a complete agent runtime rather than the one-model-turn seam required by `ModelProvider`.

The spike must answer one question with executable evidence:

- If one App Server turn accepts the complete canonical request, returns canonical assistant/tool-call output, and leaves the SDK's loop in control, implement `CodexProvider` and run provider conformance.
- If App Server necessarily owns tool iteration or other orchestration, implement `CodexRuntime`. It may reuse canonical messages, tools, results, events, and usage, but it must not claim `ModelProvider` conformance or create a hidden nested SDK loop.

`CodexBackend` is not an accepted name. The resulting ADR records the observed App Server behavior, selected seam, capability limits, and consequences before the public crate interface is called stable.

## Public seams

1. The selected `CodexProvider` or `CodexRuntime` constructor and run interface.
2. Core canonical messages, tool definitions/results, events, usage, cancellation, and errors; the integration does not create parallel public representations.
3. A process/transport seam for a controlled fake App Server in deterministic tests.

Raw JSON-RPC messages, subprocess I/O, request IDs, schema-version compatibility, and App Server notification handling remain private implementation details.

## Authentication and process ownership

- Use the installed `codex app-server` process and its normal cached ChatGPT authentication.
- Never read, parse, accept, copy, log, return, or persist cached credential contents.
- Check authentication through the supported App Server account/auth surface and return a typed actionable authentication error when login is required.
- Start a dedicated stdio App Server child by default. Own its shutdown, drain stderr safely, correlate concurrent JSON-RPC requests, and reject malformed or unmatched responses.
- Permit an explicit executable path for tests and managed installations; do not accept raw tokens.

## Test-first tracer order

1. A fake stdio App Server completes initialize, thread start, turn start, streamed text, and terminal turn state through the selected public seam.
2. Missing login becomes a typed authentication error with no credential material.
3. Canonical text input reaches the correct turn and ordered text events produce one committed assistant response.
4. Inline and URL image inputs either map losslessly or fail before starting the turn with a typed unsupported-capability error.
5. SDK tool definitions map to dynamic tools; a correlated App Server tool request executes through the core registry and returns one structured result. If App Server owns the loop, this test proves the runtime seam rather than provider conformance.
6. Invalid and unknown tool calls retain core safe-error behavior without exposing diagnostics.
7. Cancellation sends the supported interrupt/cancel operation, yields one cancelled terminal outcome, and discards late notifications.
8. Usage and rate-limit information map to canonical typed usage without inventing missing fields.
9. App Server errors, EOF, malformed JSON, unmatched IDs, unsupported schema versions, and process exit map to classified SDK errors.
10. Two parallel runs cannot cross-correlate threads, turns, tool requests, events, or cancellation.
11. Real subscription smoke: text response, SDK tool round trip, cancellation, and image support characterization against the installed Codex version.
12. The ADR is generated from the evidence, the final interface is named, and only the applicable conformance suite is required.

## Acceptance gates

- A real ChatGPT-subscription-backed text-and-tool workflow succeeds without an API key.
- The crate never accesses raw credentials and its debug/error output contains no authentication material.
- Every public type reused from core has the same meaning; no Codex-only message, tool, usage, or event model leaks to callers.
- Fake App Server tests are deterministic and run in ordinary CI without authentication.
- Live tests are explicit, ignored by default, single-account safe, and never a normal CI requirement.
- The supported Codex/App Server schema or compatibility range is explicit, with a typed error for unsupported versions.
- Experimental dynamic-tool dependence is documented accurately and isolated so it can change without destabilizing core.
- The provider-versus-runtime ADR is committed before Slice 2 is declared complete.

## Explicit non-goals

- OpenAI API, Anthropic, or another provider fallback.
- Copying Codex credentials into application configuration.
- Remote unauthenticated App Server exposure.
- Pretending built-in Codex capabilities are portable provider features.
- Weakening the core provider trait to accommodate App Server.
