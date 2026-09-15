# Agentive Rust SDK contributor instructions

### Guiding principles

- Design for agents as first-class users. Both coding agents integrating the SDK and runtime agents invoking tools must be able to discover and use the correct path from the public API, documentation, examples, diagnostics, and schemas without hidden project knowledge.
- Prefer explicit contracts over inference. Provider and tool identity, model-visible descriptions, argument meaning, schemas, policies, and failure behavior must be visible at their declaration or construction site.
- Keep one authoritative representation for each contract. Runtime validation must match published schemas, generated adapters must implement the public typed traits, and mocks must exercise the same provider boundary as production adapters.
- Macros may remove mechanical boilerplate, but must not infer protocol identity, hide orchestration, or introduce behavior that cannot be understood from the public contract.

### Implementation discipline

- Read `docs/SPEC.md`, `CONTEXT.md`, and the relevant ADRs before changing public behavior.
- Work test-first through the public seams named in the specification. Prefer one failing behavioral test followed by the smallest passing implementation.
- Keep the provider-neutral core independent of provider transports, Temporal, storage drivers, protocol adapters, and telemetry exporters.
- Treat `docs/SPEC.md` as the normative v1 contract. ADRs explain why; design sketches and research are supporting material.
- Do not weaken validation, error handling, secrecy, cancellation, replay safety, or integrity checks in the name of simplification.
