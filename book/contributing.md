# Contributing

Read [the v1 specification](https://github.com/0x63616c/agentive/blob/main/docs/SPEC.md) and relevant ADR before changing public behavior. The specification is authoritative; sketches are illustrative.

Work from public seams: complete `Agent` runs through provider and tool boundaries, provider conformance, macro compile contracts, and payload pipeline conformance. Add a failing behavioral test before the smallest passing implementation.

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
mdbook build
```

Documentation is contract work: keep snippets aligned with real public APIs and do not publish future APIs as available. The book contains application snippets, not standalone crate doctests, because mdBook does not link this workspace's crates or dependencies into its test harness. Executable API examples are validated through crate doctests and integration tests at the public provider, tool, runtime, and payload seams.
