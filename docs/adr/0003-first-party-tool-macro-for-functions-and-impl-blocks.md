# Provide a tool macro for functions and stateful impl blocks

The first release includes a first-party `#[tool]` procedural macro supporting stateless async free functions and stateful implementations. Function-only support would make examples pleasant while leaving production tools that own clients, pools, or services with avoidable adapter boilerplate. A broader agent DSL would hide orchestration and make generated behavior harder to understand.

The manual typed `Tool` trait remains the public contract and direct testing seam. The macro is only a mechanical projection from ordinary Rust into that same contract: explicit tool definition, schema, strict argument decoding, structured output serialization, and dynamic registry adaptation. It never owns provider calls, retries, approvals, or agent-loop behavior.

The authoritative normative macro, tool context, error, retry, naming, and result requirements live in the **Tools and macro** and **Tool context, errors, and retrying** sections of [the v1 specification](../SPEC.md). Keeping those details in one place prevents the ADR and implementation contract from drifting while this ADR preserves the enduring architectural choice and rationale.
