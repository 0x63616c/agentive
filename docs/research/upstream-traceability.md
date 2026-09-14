# Upstream traceability

## Baseline

This matrix maps the repository structure of `agenticenv/agent-sdk-go` v0.3.6 at commit [`d6a718f`](https://github.com/agenticenv/agent-sdk-go/tree/d6a718fac0c6d116f7264717bb5158255ba6ff61) to candidate Rust capabilities. It complements the narrative [feature map](./agents-go-sdk-feature-map.md) and prevents a package from disappearing merely because it was not prominent in the upstream README.

Disposition values are recommendations until the corresponding scope decision is accepted.

| Upstream path | Capability | Rust disposition | Earliest slice |
|---|---|---|---|
| `pkg/interfaces/llm.go` | Provider request, response, usage, streaming | Core authoring trait plus internal erasure | Foundation |
| `pkg/llm/config.go` | Provider configuration | Shared adapter conventions, not core credentials | Foundation |
| `pkg/llm/openai` | OpenAI adapter | Separate provider crate | Foundation |
| `pkg/llm/anthropic` | Anthropic adapter and prompt caching | Separate provider crate | Workflow SDK |
| `pkg/llm/gemini` | Gemini adapter | Separate provider crate | Workflow SDK |
| `pkg/llm/deepseek` | OpenAI-compatible DeepSeek adapter | Separate provider crate or compatible configuration | Workflow SDK |
| `pkg/llm/ollama` | Local/cloud Ollama adapter | Separate provider crate or compatible configuration | Workflow SDK |
| `pkg/interfaces/tool.go` | Tool contract, authorization, invocation metadata | Typed authoring trait plus erased registry adapter | Foundation |
| `pkg/tools/schema.go` | Tool JSON Schema helpers | `schemars` typed path plus raw-schema escape hatch | Foundation |
| `pkg/tools/calculator` | Example tool | Example/test fixture, not core | Foundation |
| `pkg/tools/currenttime` | Example tool | Example/test fixture, not core | Foundation |
| `pkg/tools/echo` | Example tool | Test fixture | Foundation |
| `pkg/tools/random` | Example tool | Example/test fixture, not core | Foundation |
| `pkg/tools/search` | HTTP search tool | Optional example/integration | Workflow SDK |
| `pkg/tools/weather` | HTTP weather tool | Optional example/integration | Workflow SDK |
| `pkg/tools/wikipedia` | HTTP knowledge tool | Optional example/integration | Workflow SDK |
| `pkg/agent/agent.go` | Agent façade and lifecycle | Core `Agent` and validating builder | Foundation |
| `pkg/agent/config.go` | Agent configuration | Typed builder and per-run options | Foundation |
| `pkg/agent/agent_run.go` | Run handle | Core owned control handle | Foundation |
| `pkg/agent/agent_stream.go` | Stream handle and replay offset | Core stream handle; replay capability negotiated | Foundation |
| `pkg/agent/event.go` | Public event aliases/parsers | Typed non-exhaustive event enum | Foundation |
| `pkg/agent/policy.go` | Approval policies | Core policy types; implementation in workflow slice | Workflow SDK |
| `pkg/agent/hooks.go` | Hook façade | Small middleware interface | Workflow SDK |
| `pkg/agent/tool_registry.go` | Dynamic tool registry | Snapshotting heterogeneous `DynTool` registry | Foundation |
| `pkg/agent/mcp_registry.go` | Dynamic MCP registry | MCP crate registry adapter | Workflow SDK |
| `pkg/agent/a2a_registry.go` | Dynamic A2A registry | A2A crate registry adapter | Interoperability |
| `pkg/agent/subagent_registry.go` | Dynamic sub-agent registry | Core/workflow agent-tree registry | Workflow SDK |
| `pkg/agent/subagent.go` | Delegation tool projection | Sub-agent orchestration | Workflow SDK |
| `pkg/agent/approval.go` | Human approval response path | Typed approval module | Workflow SDK |
| `pkg/agent/mcp.go` | MCP tools projected into agent tools | MCP adapter into `DynTool` | Workflow SDK |
| `pkg/agent/a2a.go` | A2A skills projected into agent tools | A2A adapter into `DynTool` | Interoperability |
| `pkg/agent/a2a_server.go` | Expose an agent over A2A | Separate server adapter | Interoperability |
| `pkg/agent/memory.go` | Memory wiring and tools | Memory orchestration policy | Knowledge |
| `pkg/agent/retriever.go` | Retriever projected as tool | Retrieval orchestration policy | Knowledge |
| `pkg/agent/worker.go` | Worker façade | Runtime-specific worker handles | Durability |
| `pkg/agent/runtime_factory.go` | Runtime selection | Explicit runtime builder seam | Durability |
| `pkg/agent/runtime/local` | Local runtime configuration | Tokio in-process runtime; journal later | Foundation / Durability |
| `pkg/agent/runtime/temporal` | Temporal public adapter | Separate runtime crate | Durability |
| `pkg/agent/runtime/restate` | Restate public adapter | Separate runtime crate | Durability |
| `internal/runtime/runtime.go` | Runtime, run, stream, worker contracts | Internal runtime interfaces and public handles | Foundation |
| `internal/runtime/base` | Shared execution operations | One runtime-independent loop module | Foundation |
| `internal/runtime/local` | In-process loop, handles, durability | Tokio adapter over shared loop | Foundation / Durability |
| `internal/runtime/temporal` | Workflow/activity implementation | Temporal adapter over serializable state machine | Durability |
| `internal/runtime/restate` | Durable invocation implementation | Restate adapter over serializable state machine | Durability |
| `internal/runtime/mocks` | Runtime mocks | Runtime conformance fixtures | Foundation |
| `internal/events` | AG-UI-shaped events and codecs | Internal event model plus optional AG-UI adapter | Foundation / Interoperability |
| `internal/eventbus` | In-memory event fan-out | Internal bounded event delivery module | Foundation |
| `internal/hooks` | LLM/tool/retrieval/memory hooks | Middleware implementation | Workflow SDK |
| `internal/store` | Key/value helper | Add only when a concrete adapter requires it | Durability |
| `internal/testing` | In-memory memory implementation | `agents-test` adapters | Knowledge |
| `internal/types` | Shared domain and wire types | Newtypes/enums in owning Rust modules | Foundation onward |
| `pkg/interfaces/conversation.go` | Conversation store contract | `ConversationStore` trait | Workflow SDK |
| `pkg/conversation/inmem` | In-memory conversation | Test/default adapter | Workflow SDK |
| `pkg/conversation/redis` | Redis conversation | Separate storage crate | Workflow SDK |
| `pkg/interfaces/memory.go` | Long-term memory contract | `MemoryStore` trait | Knowledge |
| `pkg/memory/config.go` | Recall/store/scope policy | Orchestration configuration | Knowledge |
| `pkg/memory/pgvector` | pgvector memory | Separate storage crate | Knowledge |
| `pkg/memory/weaviate` | Weaviate memory | Separate storage crate | Knowledge |
| `pkg/interfaces/retriever.go` | Retrieval contract | `Retriever` trait | Knowledge |
| `pkg/retriever/pgvector` | pgvector retrieval | Separate storage crate | Knowledge |
| `pkg/retriever/weaviate` | Weaviate retrieval | Separate storage crate | Knowledge |
| `pkg/interfaces/mcp.go` | MCP client and filters | MCP adapter seam | Workflow SDK |
| `pkg/mcp/client` | MCP stdio/HTTP client | Separate protocol crate using selected MCP SDK | Workflow SDK |
| `pkg/mcp/types.go` | MCP configuration aliases | Protocol crate types | Workflow SDK |
| `pkg/interfaces/a2a.go` | A2A client, streaming, tasks | A2A adapter seam | Interoperability |
| `pkg/a2a/client` | A2A client | Separate protocol crate | Interoperability |
| `pkg/a2a/types.go` | A2A configuration aliases | Protocol crate types | Interoperability |
| `pkg/interfaces/observability.go` | Trace, metric, and log interfaces | Emit `tracing`; exporter adapter only where needed | Workflow SDK |
| `pkg/observability` | OpenTelemetry implementation | Separate OTel crate | Workflow SDK |
| `pkg/logger` | Structured logger | Use `tracing`; no parallel logging abstraction | Foundation |
| `cli` | `agctl` run/chat/config CLI | Optional binary consuming public interfaces | Hardening |
| `examples` | End-to-end usage coverage | Compile-tested examples organized by slice | Every slice |
| `benchmarks` | Mock orchestration load harness | Criterion/custom concurrency harness | Hardening |
| `eval-harness` | Promptfoo/DeepEval integration | Optional black-box evaluation harness | Hardening |
| `docs` | Architecture, feature, runtime, production docs | Rustdoc plus book/reference documentation | Every slice |
| `.github/workflows/ci.yml` | Lint, test, coverage, build, eval | Protected required checks | Foundation |
| `.github/workflows/security.yml` | Secret, vulnerability, and code scanning | `cargo-deny`, secret scanning, dependency updates | Foundation |

## Coverage rule

Every upstream row must end in one of four explicit outcomes before the specification is complete: build in core, build as an adapter, defer to a named release slice, or reject with rationale. No row may disappear by omission.
