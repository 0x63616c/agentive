# Agentive Rust SDK

An SDK for defining and running provider-independent agent workflows in Rust.

## Language

**Provider**:
A swappable implementation that turns model requests into model responses and streaming events for the agent runtime.
_Avoid_: Backend, vendor, LLM wrapper

**Provider router**:
A provider that selects among other providers using a policy such as fallback, load balancing, or model routing.
_Avoid_: Per-run provider override, provider switch

**Agent workflow**:
A composition of agents, tools, handoffs, guardrails, and control flow that performs a task.
_Avoid_: Chain, bot script

**Tool**:
A named, described capability with a machine-readable input schema that an agent may offer to a model for invocation.
_Avoid_: Function call, plugin

**Tool definition**:
The explicit model-facing contract for a tool: its stable name, purpose, and argument schema with enough field guidance for an agent to invoke it correctly.
_Avoid_: Tool metadata, function signature

**Tool context**:
Per-invocation runtime information supplied by the SDK to a tool, separate from model-generated arguments.
_Avoid_: Arguments, global context

**Tool invocation**:
One logical request to execute a tool, retaining the same identity across attempts and recovery.
_Avoid_: Attempt, provider call

**Tool error**:
A structured tool failure with a stable application-defined code and an explicitly model-safe message, optionally retaining a private diagnostic source.
_Avoid_: Exception, raw error string

**Idempotent tool**:
A tool for which repeating the same logical invocation is explicitly safe because it produces no additional externally observable effect.
_Avoid_: Retryable tool, harmless tool

**SDK instructions**:
Versioned instructions owned by the SDK that describe the agent-runtime protocol and how runtime-generated items should be handled.
_Avoid_: User prompt, hidden prompt

**SDK core instructions**:
A minimal, always-present SDK instruction fragment containing only model-facing runtime facts and universal trust-boundary rules. It never defines application behavior, personality, or style.
_Avoid_: Default personality, universal agent prompt, safety policy

**Instruction fragment**:
A small, versioned block of SDK instructions contributed by an enabled feature to explain only that feature's model-facing protocol. Feature fragments are present only when the feature is active.
_Avoid_: Base prompt, hidden prompt, global system prompt

**Agent instructions**:
Application-authored standing instructions that define one agent's behavior across runs.
_Avoid_: System prompt, user instructions

**Run instructions**:
Application-authored instructions scoped to one run and distinct from conversation content.
_Avoid_: User message, temporary system prompt

**User message**:
End-user content added to a conversation as untrusted input for an agent to address.
_Avoid_: User instructions, prompt

**External Storage**:
The Temporal claim-check facility that stores encoded payload bytes outside Workflow History and leaves a small storage reference in their place.
_Avoid_: Reference payload codec, blob codec

**Storage Driver**:
An adapter that writes and retrieves External Storage payloads from a concrete store such as a filesystem or S3-compatible object storage.
_Avoid_: Payload codec, conversation store

**Storage ID**:
A stable, validated logical identity for a configured Storage Driver. External references carry this identity so payloads remain readable after the default write destination changes.
_Avoid_: Bucket name, storage URL, driver string

**Object ID**:
An opaque, non-content-derived identity for one immutable payload object in External Storage.
_Avoid_: Filename, checksum key, mutable blob name

**Object Key**:
A validated, versioned logical location derived from an Object ID and passed to a Storage Driver. It is relative to the driver's configured root or prefix.
_Avoid_: Object ID, absolute path, storage URL

**Payload Codec**:
An ordered, reversible transformation over a Temporal payload, such as compression. A codec may explicitly decline a payload without treating that as failure.
_Avoid_: Payload Converter, Storage Driver

**Codec ID**:
A stable, validated identity paired with a version to identify the Payload Codec responsible for an encoded envelope.
_Avoid_: Codec position, Rust type name, unvalidated metadata string

**Codec Server**:
A storage-aware HTTP service that lets authorized Temporal UI and CLI clients decode or download externally stored and encoded payloads.
_Avoid_: External Storage, worker codec
