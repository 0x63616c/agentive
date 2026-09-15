# Payload pipeline and Codec Server

`agentive-temporal` externalizes every codec-capable Temporal payload. It is infrastructure plumbing: agent code continues to use typed messages and does not receive payload references.

```rust,ignore
use agentive_temporal::{InMemoryStorage, Payload, PayloadPipeline, StorageId};

# fn example() -> Result<(), agentive_temporal::PayloadError> {
let pipeline = PayloadPipeline::single_store(InMemoryStorage::new(), StorageId::new("test-store")?)?;
let encoded = pipeline.encode(Payload::new(b"hello".to_vec()))?;
assert!(encoded.is_external_reference());
assert_eq!(pipeline.decode(encoded)?.data(), b"hello");
# Ok(())
# }
```

For production, construct `FilesystemStorage` with an application-owned root and reuse one immutable `PayloadPipeline` configuration in every Temporal client, worker, and Codec Server. Objects are random, immutable, and validated by size, SHA-256, codec stack, and pipeline fingerprint before decoding; malformed or unknown envelopes fail closed. There is no inline-size threshold: every codec-capable Temporal payload reaches the external-storage stage.

Add `ZstdCodec::default()` through `PayloadPipeline::builder()` for beneficial compression. Codecs encode in declaration order and decode in reverse. `CodecServer` is a framework-neutral Tower handler; `axum_router` is behind the `axum` feature. It only decodes `POST /decode` requests through its configured pipeline. Applications own authentication, authorization, TLS, listener binding, and operational retention. V1 has an in-memory test fake and filesystem storage only—no S3 and no automatic object deletion.
