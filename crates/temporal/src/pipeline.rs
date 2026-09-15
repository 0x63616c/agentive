//! Immutable codec-chain orchestration and mandatory external storage.

use crate::{
    PayloadError,
    codec::{CodecOutcome, PayloadCodec},
    payload::{
        CodecUse, ExternalReference, Payload, PayloadLimits, PipelineFingerprint, REFERENCE_MARKER,
    },
    storage::{ObjectId, ObjectKey, StorageDriver, StorageId},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Builder for an immutable always-on external-storage payload pipeline.
#[derive(Debug, Default)]
pub struct PayloadPipelineBuilder {
    storages: BTreeMap<StorageId, Arc<dyn StorageDriver>>,
    default_storage: Option<StorageId>,
    codecs: Vec<Arc<dyn PayloadCodec>>,
    limits: PayloadLimits,
}
impl PayloadPipelineBuilder {
    #[must_use]
    /// Starts an empty pipeline builder with v1 limits.
    pub fn new() -> Self {
        Self {
            limits: PayloadLimits::default(),
            ..Self::default()
        }
    }
    /// Registers one named storage driver. Duplicate IDs are rejected.
    /// # Errors
    /// Returns an error if an ID is already registered.
    pub fn storage(
        mut self,
        id: StorageId,
        driver: impl StorageDriver,
    ) -> Result<Self, PayloadError> {
        if self.storages.insert(id, Arc::new(driver)).is_some() {
            return Err(PayloadError::InvalidInput {
                kind: "storage registry",
                reason: "storage id is already registered",
            });
        }
        Ok(self)
    }
    #[must_use]
    /// Selects the registered destination for new immutable objects.
    pub fn default_storage(mut self, id: StorageId) -> Self {
        self.default_storage = Some(id);
        self
    }
    #[must_use]
    /// Appends an ordered transform codec.
    pub fn codec(mut self, codec: impl PayloadCodec + 'static) -> Self {
        self.codecs.push(Arc::new(codec));
        self
    }
    #[must_use]
    /// Applies bounded read and decompression limits.
    pub fn limits(mut self, limits: PayloadLimits) -> Self {
        self.limits = limits;
        self
    }
    /// Validates ownership and constructs an immutable pipeline.
    /// # Errors
    /// Returns errors for missing storage, unknown writers, or overlapping codec ownership.
    pub fn build(self) -> Result<PayloadPipeline, PayloadError> {
        let default_storage = self.default_storage.ok_or(PayloadError::InvalidInput {
            kind: "storage registry",
            reason: "a default writer is required",
        })?;
        if !self.storages.contains_key(&default_storage) {
            return Err(PayloadError::InvalidInput {
                kind: "storage registry",
                reason: "default writer is not registered",
            });
        }
        let mut owners = BTreeSet::new();
        for codec in &self.codecs {
            let descriptor = codec.descriptor()?;
            for version in descriptor.decode_versions() {
                if !owners.insert((descriptor.id().clone(), *version)) {
                    return Err(PayloadError::InvalidInput {
                        kind: "codec registry",
                        reason: "codec decode ownership overlaps",
                    });
                }
            }
        }
        Ok(PayloadPipeline {
            fingerprint: PipelineFingerprint::calculate(&default_storage, &self.codecs)?,
            storages: self.storages,
            default_storage,
            codecs: self.codecs,
            limits: self.limits,
        })
    }
}

/// An immutable codec chain and mandatory External Storage stage.
#[derive(Debug, Clone)]
pub struct PayloadPipeline {
    storages: BTreeMap<StorageId, Arc<dyn StorageDriver>>,
    default_storage: StorageId,
    codecs: Vec<Arc<dyn PayloadCodec>>,
    limits: PayloadLimits,
    fingerprint: PipelineFingerprint,
}
impl PayloadPipeline {
    /// Creates a one-store pipeline with no transforms.
    /// # Errors
    /// Returns an error only for invalid construction.
    pub fn single_store(driver: impl StorageDriver, id: StorageId) -> Result<Self, PayloadError> {
        Self::builder()
            .storage(id.clone(), driver)?
            .default_storage(id)
            .build()
    }
    #[must_use]
    /// Starts an empty pipeline builder.
    pub fn builder() -> PayloadPipelineBuilder {
        PayloadPipelineBuilder::new()
    }
    #[must_use]
    /// Returns the configuration fingerprint recorded in new references.
    pub fn fingerprint(&self) -> &PipelineFingerprint {
        &self.fingerprint
    }
    #[must_use]
    /// Returns this pipeline's immutable size limits.
    pub fn limits(&self) -> PayloadLimits {
        self.limits
    }
    /// Serializes, transforms, and always externalizes one payload.
    /// # Errors
    /// Returns typed serialization, codec, or storage errors.
    pub fn encode(&self, payload: Payload) -> Result<Payload, PayloadError> {
        let mut bytes = serde_json::to_vec(&payload)?;
        if bytes.len() > self.limits.decoded_payload_bytes() {
            return Err(PayloadError::SizeLimit {
                kind: "decoded payload",
            });
        }
        let mut used = Vec::with_capacity(self.codecs.len());
        for codec in &self.codecs {
            match codec.encode(&bytes)? {
                CodecOutcome::Applied(transformed) => {
                    bytes = transformed;
                    let descriptor = codec.descriptor()?;
                    used.push(CodecUse {
                        id: descriptor.id().clone(),
                        version: descriptor.encode_version(),
                    });
                }
                CodecOutcome::NotHandled => {}
            }
        }
        if bytes.len() > self.limits.batch_bytes() {
            return Err(PayloadError::SizeLimit { kind: "batch" });
        }
        let object_id = ObjectId::random();
        let key = ObjectKey::v1_for(object_id);
        self.storages
            .get(&self.default_storage)
            .ok_or(PayloadError::UnknownStorage)?
            .create_immutable(&key, &bytes)?;
        let reference = ExternalReference::new(
            self.default_storage.clone(),
            object_id,
            &bytes,
            used,
            &self.fingerprint,
        )?;
        Ok(Payload::new(serde_json::to_vec(&reference)?)
            .with_metadata("encoding", REFERENCE_MARKER.as_bytes().to_vec()))
    }
    /// Resolves an external reference, verifies it, reverses transforms, and deserializes. Plain payloads pass through unchanged.
    /// # Errors
    /// Returns typed reference, integrity, codec, or storage errors.
    pub fn decode(&self, payload: Payload) -> Result<Payload, PayloadError> {
        if !payload.is_external_reference() {
            return if payload.claims_external_reference() {
                Err(PayloadError::MalformedReference)
            } else {
                Ok(payload)
            };
        }
        let reference: ExternalReference =
            serde_json::from_slice(payload.data()).map_err(|_| PayloadError::MalformedReference)?;
        reference.validate(self.limits, &self.fingerprint)?;
        let encoded_size =
            usize::try_from(reference.encoded_size).map_err(|_| PayloadError::SizeLimit {
                kind: "stored payload",
            })?;
        let mut bytes = self
            .storages
            .get(&reference.storage_id)
            .ok_or(PayloadError::UnknownStorage)?
            .read(&ObjectKey::v1_for(reference.object_id), encoded_size)?;
        if bytes.len() != encoded_size || !reference.matches_bytes(&bytes) {
            return Err(PayloadError::IntegrityMismatch);
        }
        for used in reference.codecs.iter().rev() {
            let codec = self
                .codecs
                .iter()
                .find(|codec| {
                    codec.descriptor().is_ok_and(|descriptor| {
                        descriptor.id() == &used.id
                            && descriptor.accepts_decode_version(used.version)
                    })
                })
                .ok_or(PayloadError::UnknownCodec)?;
            bytes = codec.decode(used.version, &bytes, self.limits.decoded_payload_bytes())?;
            if bytes.len() > self.limits.decoded_payload_bytes() {
                return Err(PayloadError::SizeLimit {
                    kind: "decoded payload",
                });
            }
        }
        serde_json::from_slice(&bytes).map_err(|_| PayloadError::MalformedReference)
    }
}
