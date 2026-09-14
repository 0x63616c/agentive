# Agents Rust SDK

An idiomatic, provider-independent Rust SDK for building testable and durable agent workflows.

The project is currently entering test-first implementation. The authoritative design is in [the v1 specification](docs/SPEC.md), with architectural rationale in [the ADRs](docs/adr/).

Initial scope:

- deterministic `ScriptedProvider`
- ChatGPT subscription-backed Codex integration
- typed tools and a first-party `#[tool]` macro
- text and image conversations
- run control, streaming events, limits, and usage
- explicit sub-agent delegation
- Temporal durability
- filesystem-backed Temporal External Storage and Codec Server

OpenAI API, Anthropic, MCP, S3, approvals, and a separate conversation store are intentionally deferred.
