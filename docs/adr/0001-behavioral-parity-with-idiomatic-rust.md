# Preserve behavior, not Go API shape

The SDK will cover the core capabilities and observable semantics of `agenticenv/agent-sdk-go`, but its public API, ownership model, concurrency model, errors, traits, and module boundaries will be designed idiomatically for Rust. A direct port would make comparison easier but would permanently import Go-specific abstractions and ergonomics into a foundational Rust library.
