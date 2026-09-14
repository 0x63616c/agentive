# Usage, model context, and large durable payloads

## Status

Provider-reported token usage in completed run results is accepted. Shipping always-on Temporal External Storage and a storage-aware Codec Server library without exposing payload handling through the agent interface is accepted. Context-limit policy, retention, and codec composition are being designed.

## Accepted usage semantics

Every completed run reports aggregate token usage across every model call in that run. The canonical vocabulary distinguishes input, output, cached-input, reasoning, and provider-reported total tokens. A provider that does not report a measurement produces `unknown`, not zero. Aggregation preserves whether the total is complete, and per-call usage remains observable through typed execution records and events.

Illustrative shape; exact Rust names remain proposed:

```rust
pub struct RunUsage {
    pub aggregate: TokenUsage,
    pub complete: bool,
    pub model_calls: Vec<ModelCallUsage>,
}

pub struct TokenUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cached_input: Option<u64>,
    pub reasoning: Option<u64>,
    pub provider_total: Option<u64>,
}
```

## Three separate concerns

1. Usage accounting records what a provider reports after a model call.
2. Context planning determines whether the next canonical model request fits the selected model's context window and what to do when it does not.
3. Payload transport determines how canonical state crosses process and durable-runtime seams without putting large byte strings into workflow history.

A payload codec or external-storage data converter can solve transport size. It cannot make a conversation fit a model context window because the model must ultimately receive the selected content in readable form.

## Go reference behavior

The Go Temporal runtime places `Messages []Message` directly in each LLM activity input and in the state carried through Continue-As-New. When save-on-iteration is enabled, completed message batches are written through a conversation activity and cleared from workflow-local state; the LLM activity then reloads recent conversation messages from the configured store. This reduces some activity arguments but does not provide a general large-payload codec or immutable context-snapshot protocol.

## Proposed Rust direction

- Preserve the complete conversation rather than silently retaining only the last fixed number of messages.
- Keep provider-neutral messages ordinary typed Rust values. Do not force a Temporal-shaped payload or conversation-snapshot reference into core.
- Make large-payload handling transparent at the durable-runtime seam. A Temporal data-conversion pipeline may serialize, compress or encrypt, and externally store payloads above a threshold, leaving a small claim-check reference in Workflow History while activities receive the original decoded Rust values.
- Treat byte transformation and external storage as distinct responsibilities even when one configured pipeline composes them: a payload codec transforms bytes; an external payload store performs the claim check.
- Put Temporal data-converter, external-storage, and codec-server helpers in the Temporal integration crate rather than provider-neutral core. V1 proves the storage seam with a filesystem implementation; object-storage drivers can follow without making every runtime depend on them.
- Default v1 context behavior to all-or-error if automatic truncation, summarization, and compaction are not enabled: send the complete requested context when it fits, otherwise return a typed preflight context-limit error.
- Report estimated context occupancy before a call and provider-reported occupancy after it when the provider exposes enough information. The report must distinguish exact, estimated, and unknown values.

## Accepted Temporal integration boundary

The SDK distribution will include Temporal External Storage, an initial local/shared-filesystem Storage Driver, and a storage-aware Codec Server library. These are configured on Temporal clients and workers through the Temporal data-conversion seam. Every codec-capable Temporal payload is externalized; there is no inline-size threshold or dual representation. Agent builders, canonical messages, workflow inputs, and activity functions remain unaware of references and continue to use ordinary typed Rust values. S3-compatible and other drivers are future implementations of the same seam, not v1 scope.

The implementation uses Temporal's canonical vocabulary: External Storage for claim-check offloading, Storage Driver for the concrete external store, Payload Codec for byte transformations such as compression or encryption, and Codec Server for UI/CLI access. `ReferencePayloadCodec` is rejected because it conflates byte transformation with external storage.

The Codec Server is delivered as a reusable library handler that applications mount and secure inside their own server. The SDK does not ship or operate a standalone executable.

Storage Drivers are registered under validated `StorageId` values. Exactly one registered driver is the default destination for new writes, while any number may remain available for reads. Every external reference records its `StorageId`, allowing deployments to migrate writes to a new store without rewriting existing Workflow History. Unknown IDs fail explicitly rather than falling back to another driver. A single-store constructor supplies an internal default identity so the common path exposes neither a registry nor a magic string.

Each stored payload is addressed by an opaque, non-content-derived `ObjectId` and is create-only. The external reference also records the encoded byte length and a cryptographic checksum. A read succeeds only after both values are verified. Retrying creation of an existing object with the same checksum is success; encountering the same object identity with different bytes is an integrity error. The SDK never silently overwrites an object, substitutes another object, or returns corrupt bytes. Content-addressed object names are not the default because storage paths should not reveal equality between plaintext or merely compressed payloads.

The shared External Storage module, rather than each Storage Driver, maps a reference version and `ObjectId` to a validated relative `ObjectKey`. The accepted v1 layout is `v1/objects/<first-three-hex>/<full-object-id>`. Three hexadecimal characters provide 4,096 possible top-level shards, created only as objects require them. The filesystem driver joins this key beneath its configured root. Drivers cannot accept absolute paths or traversal segments through `ObjectKey`. References retain the version and `ObjectId`, so the SDK can derive the same key without exposing physical storage locations; future storage drivers map the same logical key into their own namespace.

Retention is operator-owned. The SDK never automatically deletes stored payloads and does not ship a garbage collector in v1 because it cannot prove that no live, retained, or archived Workflow History still references an object. Filesystem deployments use an external cleanup policy. Documentation requires configured storage retention to outlive all applicable Temporal history and archive retention. A reference may carry creation metadata for operations, but expiry never changes decode behavior: an absent object is always a typed missing-object error.

Payload Codec order is explicit and reversible. Encoding runs the declared codecs from first to last; decoding runs the same codecs from last to first. The SDK does not infer or silently reorder dependencies. V1 ships compression but no first-party authenticated-encryption codec or key-management implementation. Its built-in path is serialization, beneficial compression, checksum and encoded-size calculation, then External Storage. On read, External Storage fetch and checksum/size verification precede decompression and deserialization. The chain remains open to application-supplied codecs. If an application supplies encryption, compression conventionally precedes it because encrypted bytes are intentionally indistinguishable from random data and therefore do not compress usefully. The checksum covers stored bytes for corruption detection and idempotency; it is not a security boundary. Deployments without an application encryption codec rely on transport security, storage access control, and storage-side encryption, and the documentation must not imply end-to-end confidentiality.

Codec participation is explicit. Encoding and decoding return either an applied payload or an unchanged not-handled payload; declining to handle a payload is not an error. An applied codec wraps the result with a validated stable `CodecId` and typed format version so reverse decoding selects by declared ownership rather than position or Rust type name. Plain Temporal payloads may pass through the complete chain. By contrast, an SDK-owned encoded envelope must be fully consumed: an unavailable codec, unsupported version, malformed envelope, or residual SDK envelope is a typed terminal decoding error. The runtime must not guess, fall back to plaintext, or deliver encoded bytes to application code.

A codec descriptor declares exactly one version used for new encoding and a set of versions accepted for decoding. This lets a replacement encoder write a new format while continuing to read durable objects written by older deployments. Chain construction rejects any overlap where two codecs claim the same `(CodecId, CodecVersion)` for decoding. Registration order never resolves ownership ambiguity.

## Open decisions

1. Whether all-or-error is the accepted v1 default context policy.
2. Whether image inputs accept inline bytes, external references, or both.
3. How provider limits and token estimation are declared, overridden, and tested.
4. Exact encoded-envelope representation and codec identity/version wire format.
5. Codec Server handler framework seam and secure defaults.
