//! Public payload-substrate contract tests.

#![allow(clippy::expect_used)] // Fixture assertions make failures legible.

use std::sync::{Arc, Mutex};

use agentive_temporal::{
    CodecServer, FilesystemStorage, InMemoryStorage, ObjectId, ObjectKey, Payload, PayloadPipeline,
    StorageDriver, StorageId, ZstdCodec,
};
use proptest::prelude::*;
use temporalio_common::data_converters::{DataConverter, SerializationContextData};

#[derive(Debug, Clone, Default)]
struct ThreadRecordingStorage {
    inner: InMemoryStorage,
    create_thread: Arc<Mutex<Option<std::thread::ThreadId>>>,
}

impl StorageDriver for ThreadRecordingStorage {
    fn create_immutable(
        &self,
        key: &ObjectKey,
        bytes: &[u8],
    ) -> Result<(), agentive_temporal::PayloadError> {
        *self.create_thread.lock().expect("test mutex") = Some(std::thread::current().id());
        self.inner.create_immutable(key, bytes)
    }

    fn read(
        &self,
        key: &ObjectKey,
        max_bytes: usize,
    ) -> Result<Vec<u8>, agentive_temporal::PayloadError> {
        self.inner.read(key, max_bytes)
    }
}

#[test]
fn in_memory_pipeline_always_externalizes_and_round_trips() -> Result<(), Box<dyn std::error::Error>>
{
    let storage = InMemoryStorage::new();
    let pipeline = PayloadPipeline::single_store(storage, StorageId::new("test-store")?)?;
    let encoded = pipeline.encode(Payload::new(b"a small payload".to_vec()))?;

    assert!(encoded.is_external_reference());
    assert_eq!(pipeline.decode(encoded)?.data(), b"a small payload");
    Ok(())
}

#[test]
fn codec_server_uses_the_same_public_pipeline() -> Result<(), Box<dyn std::error::Error>> {
    let pipeline =
        PayloadPipeline::single_store(InMemoryStorage::new(), StorageId::new("server")?)?;
    let encoded = pipeline.encode(Payload::new(b"codec server".to_vec()))?;
    let server = CodecServer::new(pipeline);
    assert_eq!(server.decode_payload(encoded)?.data(), b"codec server");
    Ok(())
}

#[tokio::test]
async fn temporal_data_converter_always_externalizes_and_decodes_with_the_same_pipeline()
-> Result<(), Box<dyn std::error::Error>> {
    let pipeline =
        PayloadPipeline::single_store(InMemoryStorage::new(), StorageId::new("temporal")?)?;
    let converter: DataConverter = agentive_temporal::temporal_data_converter(pipeline);
    let context = SerializationContextData::None;
    let input = String::from("durable payload");

    let encoded = converter.to_payload(&context, &input).await?;
    assert_eq!(
        encoded.metadata.get("encoding").map(Vec::as_slice),
        Some(b"agentive.io/external-storage-v1".as_slice())
    );
    let decoded: String = converter.from_payload(&context, encoded).await?;
    assert_eq!(decoded, input);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn temporal_converter_moves_sync_storage_work_off_the_runtime_thread()
-> Result<(), Box<dyn std::error::Error>> {
    let storage = ThreadRecordingStorage::default();
    let recorded = Arc::clone(&storage.create_thread);
    let pipeline = PayloadPipeline::single_store(storage, StorageId::new("off-runtime")?)?;
    let converter = agentive_temporal::temporal_data_converter(pipeline);
    let current = std::thread::current().id();

    let _ = converter
        .to_payload(&SerializationContextData::None, &"sync storage")
        .await?;

    assert_ne!(*recorded.lock().expect("test mutex"), Some(current));
    Ok(())
}

#[test]
fn compressed_pipeline_round_trips_and_random_ids_use_the_locked_key_layout()
-> Result<(), Box<dyn std::error::Error>> {
    let id = ObjectId::random();
    assert_eq!(id.to_hex().len(), 32);
    assert!(
        id.to_hex()
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    );
    assert_eq!(
        ObjectKey::v1_for(id).as_str(),
        format!("v1/objects/{}/{id}", &id.to_hex()[..3], id = id.to_hex())
    );

    let pipeline = PayloadPipeline::builder()
        .storage(StorageId::new("memory")?, InMemoryStorage::new())?
        .default_storage(StorageId::new("memory")?)
        .codec(ZstdCodec::default())
        .build()?;
    let payload = Payload::new(vec![b'a'; 20_000]);
    assert_eq!(pipeline.decode(pipeline.encode(payload.clone())?)?, payload);
    Ok(())
}

#[test]
fn immutable_creates_are_idempotent_and_conflicts_are_typed()
-> Result<(), Box<dyn std::error::Error>> {
    let storage = InMemoryStorage::new();
    let key = ObjectKey::v1_for(ObjectId::random());
    storage.create_immutable(&key, b"same")?;
    storage.create_immutable(&key, b"same")?;
    assert!(matches!(
        storage.create_immutable(&key, b"different"),
        Err(agentive_temporal::PayloadError::ImmutableConflict)
    ));
    Ok(())
}

proptest! {
    #[allow(clippy::expect_used)] // proptest's TestCaseError does not preserve library errors.
    #[test]
    fn payloads_round_trip_through_external_storage(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let pipeline = PayloadPipeline::single_store(InMemoryStorage::new(), StorageId::new("property").expect("test constant"))
            .expect("test pipeline");
        let payload = Payload::new(data);
        let encoded = pipeline.encode(payload.clone()).expect("encoding should work");
        prop_assert!(encoded.is_external_reference());
        prop_assert_eq!(pipeline.decode(encoded).expect("decoding should work"), payload);
    }
}

#[test]
fn filesystem_driver_round_trips_through_the_public_pipeline()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let storage = FilesystemStorage::new(directory.path())?;
    let pipeline = PayloadPipeline::single_store(storage, StorageId::new("filesystem")?)?;
    let input = Payload::new(vec![b'x'; 4_096]);

    let encoded = pipeline.encode(input.clone())?;
    assert!(encoded.is_external_reference());
    assert_eq!(pipeline.decode(encoded)?, input);
    Ok(())
}

#[test]
fn unknown_agentive_external_storage_envelopes_fail_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let pipeline =
        PayloadPipeline::single_store(InMemoryStorage::new(), StorageId::new("strict")?)?;
    let payload = Payload::new(b"not a supported reference".to_vec())
        .with_metadata("encoding", b"agentive.io/external-storage-v2".to_vec());

    assert!(matches!(
        pipeline.decode(payload),
        Err(agentive_temporal::PayloadError::MalformedReference)
    ));
    Ok(())
}

#[cfg(feature = "axum")]
#[tokio::test]
async fn axum_adapter_honors_pipeline_limit_above_axums_default()
-> Result<(), Box<dyn std::error::Error>> {
    use agentive_temporal::PayloadLimits;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    // `Vec<u8>` is JSON-encoded as numbers, so 3 MiB of padding expands well
    // past its raw byte count while still testing the configured admission path.
    let limit = 20 * 1024 * 1024;
    let pipeline = PayloadPipeline::builder()
        .storage(StorageId::new("axum")?, InMemoryStorage::new())?
        .default_storage(StorageId::new("axum")?)
        .limits(PayloadLimits::new(limit + 1024, limit + 1024)?)
        .build()?;
    let encoded = pipeline
        .encode(Payload::new(b"within the decoded payload limit".to_vec()))?
        // Reference metadata is part of the wire body but only `encoding`
        // selects the strict external-reference decoder.
        .with_metadata("padding", vec![b'x'; 3 * 1024 * 1024]);
    let body = serde_json::to_vec(&agentive_temporal::CodecServerRequest::new(encoded))?;
    assert!(body.len() > 2 * 1024 * 1024);

    let response = agentive_temporal::axum_router(CodecServer::new(pipeline))
        .oneshot(
            Request::post("/decode")
                .header("content-type", "application/octet-stream")
                .body(Body::from(body))?,
        )
        .await?;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    Ok(())
}
