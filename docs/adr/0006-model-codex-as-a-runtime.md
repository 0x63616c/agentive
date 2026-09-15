# Model Codex App Server as a runtime, not a provider

## Decision

Agentive exposes the locally authenticated Codex integration as
`agentive_codex::CodexRuntime`. It does not implement the provider-neutral
`ModelProvider` trait and is not named `CodexProvider`.

`CodexRuntime` starts one isolated `codex app-server` process per run, relies on
that process's existing ChatGPT subscription session, maps Agentive text, URL
images, dynamic tools, usage, cancellation, and failures onto the App Server
protocol, and never reads or accepts cached credentials.

## Context

`ModelProvider` represents one model turn inside Agentive's own loop. Codex App
Server owns a richer agent loop: threads, turns, streamed items, tool callbacks,
approvals, and interruption. Hiding it behind `ModelProvider` would nest two
orchestrators and make retry, tool, cancellation, and usage ownership ambiguous.

The conformance spike and fake transport tests confirmed that the App Server
boundary can be integrated honestly as a complete runtime. The opt-in live smoke
test is the manual release gate for the installed subscription path; it introduces
no API-key path and is not run by hosted CI because it requires a developer's
locally authenticated Codex installation. That gate passed end to end on
2026-09-14 using `gpt-5.6-luna`, including one dynamic tool callback and usage.

## Consequences

- Applications choose either Agentive's provider-neutral loop or the Codex
  runtime explicitly; neither masquerades as the other.
- Core provider conformance tests remain strict and contain no Codex exception.
- One Codex run owns one App Server session, so parallel runs cannot mix thread
  or turn identifiers.
- Inline image bytes are rejected before process startup until the App Server
  exposes a lossless supported representation; URL images remain supported.
- Dynamic tool support is experimental upstream and stays isolated in the
  integration crate.
- A future raw Codex model transport may implement `ModelProvider`, but only if
  it performs exactly one model turn without inheriting App Server orchestration.

## Evidence

The characterized protocol surface, fake-server coverage, live-test command, and
OpenClaw comparison are recorded in
[`crates/codex/docs/conformance.md`](../../crates/codex/docs/conformance.md).
