# Compatibility, security, and non-goals

## Compatibility

The workspace MSRV is Rust 1.94 and edition 2024. Public crates are currently `0.1.0`; upgrade deliberately and run your integration suite. Pipeline fingerprints make payload configuration mismatches explicit.

## Security defaults

- Core instructions treat user and external content as untrusted.
- Tool errors expose only stable model-safe contracts; private sources stay private.
- Filesystem storage rejects non-relative object keys; objects are immutable and checksum-verified.
- Codex uses the existing local subscription and never asks you to copy credentials.

You own authorization, tool access control, secret handling, TLS, Codec Server authentication, retention, and rate limits.

## Non-goals

V1 does not ship OpenAI API or Anthropic adapters, MCP/A2A/AG-UI, a cross-run conversation store, automatic truncation, summarization, or compaction, retrieval/vector memory, S3, human approvals, audio/video/generic-file messages, streaming tool results, payload encryption/key management, garbage collection, price budgets, or telemetry exporters. The shipped Temporal runtime is intentionally narrow: it drives the canonical state machine through activities and requires the always-external payload pipeline; it is not a conversation store or a replacement provider loop.
