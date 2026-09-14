# Slice 4: filesystem payload substrate

## Outcome

Temporal-compatible payload bytes can pass through a deterministic codec pipeline and mandatory External Storage stage, be stored immutably on a filesystem, and be decoded through both library code and an embeddable Codec Server handler.

This slice builds and proves the payload substrate before the Temporal workflow adapter depends on it.

## Public seams

1. `PayloadPipeline` encodes and decodes typed payload envelopes.
2. `StorageDriver` stores and retrieves immutable bytes by validated `ObjectKey`; in-memory and filesystem adapters satisfy it.
3. The Codec Server Tower interface accepts Temporal-compatible decode requests using the same immutable pipeline.

Physical paths, JSON parsing, compression implementation, checksumming, registry lookup, and HTTP framework wiring remain private.

## Test-first tracer order

1. An in-memory driver round-trips an externalized payload through one `PayloadPipeline`.
2. The filesystem adapter passes the same Storage Driver conformance case in a temporary directory.
3. New objects receive random 128-bit IDs rendered as 32 lowercase hexadecimal characters and the shared `v1/objects/<first-three-hex>/<full-object-id>` key.
4. Repeating an identical create succeeds; different bytes under the same ID conflict; reads verify length and SHA-256.
5. Absolute, traversal, malformed, duplicate-field, unknown-field, unknown-storage, and unsupported-reference inputs fail closed with typed errors.
6. Zstd compression applies only when beneficial, records a typed codec ID/version, and reverses correctly before deserialization.
7. A transform may return not handled, but the mandatory External Storage stage still replaces the payload with a reference.
8. Multiple codecs encode in declaration order and decode in reverse; overlapping decode ownership is rejected.
9. A new default writer can coexist with historical readers, and old references continue to select their recorded Storage ID.
10. Configured streaming read, batch, and decompression limits reject oversized content without allocating the declared size blindly.
11. Pipeline construction produces a stable fingerprint; incompatible pipelines and unavailable historical decoders fail explicitly.
12. The framework-neutral Codec Server handler decodes an external reference using the same pipeline; the Axum convenience adapter adds no different semantics.
13. Corruption, missing files, partial I/O, transient unavailability, and permission failure produce the documented retry disposition without hidden driver retry.

## Acceptance gates

- One shared conformance suite passes against both in-memory and filesystem Storage Drivers.
- Codec and envelope property tests cover round trips and hostile inputs.
- Stored objects are never overwritten or automatically deleted.
- Every codec-capable input is externalized regardless of size or compression outcome.
- Codec Server tests use the public HTTP/Tower seam and a temporary filesystem, not private storage inspection.
- The crate contains no S3 client, encryption/key-management system, network listener, application authentication policy, or Temporal workflow runtime.
- All prior slice gates remain green.

## Explicit non-goals

- S3 or another object store.
- Encryption, key management, garbage collection, or retention automation.
- Owning a web server process, TLS, authentication, or authorization.
- Context-window management or conversation persistence.
