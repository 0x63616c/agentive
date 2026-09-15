//! Payload values and the strict external-storage reference format.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    PayloadError,
    codec::{CodecId, CodecVersion},
    storage::{ObjectId, StorageId, sha256},
};

pub(crate) const REFERENCE_MARKER: &str = "agentive.io/external-storage-v1";
const REFERENCE_MARKER_PREFIX: &[u8] = b"agentive.io/external-storage-";
pub(crate) const REFERENCE_VERSION: u8 = 1;
const DEFAULT_MAX_DECODED_BYTES: usize = 64 * 1024 * 1024;

/// A Temporal-compatible payload represented without a Temporal runtime dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    data: Vec<u8>,
    metadata: BTreeMap<String, Vec<u8>>,
}

impl Payload {
    #[must_use]
    /// Creates a payload with no metadata.
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            metadata: BTreeMap::new(),
        }
    }

    #[must_use]
    /// Borrows the payload's raw data.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    #[must_use]
    /// Borrows the payload's application metadata.
    pub fn metadata(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.metadata
    }

    /// Adds one application metadata entry before payload encoding.
    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: Vec<u8>) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    pub(crate) fn with_metadata_map(mut self, metadata: BTreeMap<String, Vec<u8>>) -> Self {
        self.metadata = metadata;
        self
    }

    #[must_use]
    /// Returns whether this is an SDK External Storage reference payload.
    pub fn is_external_reference(&self) -> bool {
        self.metadata
            .get("encoding")
            .is_some_and(|value| value.as_slice() == REFERENCE_MARKER.as_bytes())
    }

    pub(crate) fn claims_external_reference(&self) -> bool {
        self.metadata
            .get("encoding")
            .is_some_and(|value| value.starts_with(REFERENCE_MARKER_PREFIX))
    }
}

/// Limits applied before and during external retrieval and decompression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayloadLimits {
    decoded_payload_bytes: usize,
    batch_bytes: usize,
}

impl Default for PayloadLimits {
    fn default() -> Self {
        Self {
            decoded_payload_bytes: DEFAULT_MAX_DECODED_BYTES,
            batch_bytes: DEFAULT_MAX_DECODED_BYTES,
        }
    }
}

impl PayloadLimits {
    /// Creates limits. Both bounds must be nonzero.
    ///
    /// # Errors
    /// Returns an error for a zero bound.
    pub const fn new(
        decoded_payload_bytes: usize,
        batch_bytes: usize,
    ) -> Result<Self, PayloadError> {
        if decoded_payload_bytes == 0 || batch_bytes == 0 {
            Err(PayloadError::InvalidInput {
                kind: "payload limits",
                reason: "limits must be nonzero",
            })
        } else {
            Ok(Self {
                decoded_payload_bytes,
                batch_bytes,
            })
        }
    }

    #[must_use]
    /// Returns the decoded-payload byte bound.
    pub const fn decoded_payload_bytes(self) -> usize {
        self.decoded_payload_bytes
    }
    #[must_use]
    /// Returns the aggregate external-read byte bound.
    pub const fn batch_bytes(self) -> usize {
        self.batch_bytes
    }
}

/// A stable SHA-256 identity of one immutable pipeline configuration.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PipelineFingerprint([u8; 32]);

impl PipelineFingerprint {
    /// Parses the canonical lowercase hexadecimal fingerprint stored in a reference.
    ///
    /// # Errors
    /// Returns an error for a non-canonical fingerprint.
    pub fn parse(value: &str) -> Result<Self, PayloadError> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(PayloadError::InvalidInput {
                kind: "pipeline fingerprint",
                reason: "must be 64 lowercase hexadecimal characters",
            });
        }
        let mut bytes = [0_u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| {
                PayloadError::InvalidInput {
                    kind: "pipeline fingerprint",
                    reason: "must be 64 lowercase hexadecimal characters",
                }
            })?;
        }
        Ok(Self(bytes))
    }
    pub(crate) fn calculate(
        default_storage: &StorageId,
        codecs: &[std::sync::Arc<dyn crate::PayloadCodec>],
    ) -> Result<Self, PayloadError> {
        let mut hasher = Sha256::new();
        hasher.update(b"agentive-temporal-pipeline-v1\0");
        hasher.update(default_storage.as_str().as_bytes());
        for codec in codecs {
            let descriptor = codec.descriptor()?;
            hasher.update(b"\0");
            hasher.update(descriptor.id().as_str().as_bytes());
            hasher.update(descriptor.encode_version().get().to_be_bytes());
            for version in descriptor.decode_versions() {
                hasher.update(version.get().to_be_bytes());
            }
        }
        Ok(Self(hasher.finalize().into()))
    }

    #[must_use]
    /// Renders the stable SHA-256 fingerprint as lowercase hexadecimal.
    pub fn to_hex(&self) -> String {
        hex(self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CodecUse {
    pub(crate) id: CodecId,
    pub(crate) version: CodecVersion,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExternalReference {
    version: u8,
    pub(crate) storage_id: StorageId,
    pub(crate) object_id: ObjectId,
    pub(crate) encoded_size: u64,
    sha256: String,
    pub(crate) codecs: Vec<CodecUse>,
    pipeline_fingerprint: String,
}

impl ExternalReference {
    pub(crate) fn new(
        storage_id: StorageId,
        object_id: ObjectId,
        bytes: &[u8],
        codecs: Vec<CodecUse>,
        fingerprint: &PipelineFingerprint,
    ) -> Result<Self, PayloadError> {
        Ok(Self {
            version: REFERENCE_VERSION,
            storage_id,
            object_id,
            encoded_size: u64::try_from(bytes.len()).map_err(|_| PayloadError::SizeLimit {
                kind: "stored payload",
            })?,
            sha256: hex(sha256(bytes)),
            codecs,
            pipeline_fingerprint: fingerprint.to_hex(),
        })
    }

    pub(crate) fn validate(
        &self,
        limits: PayloadLimits,
        fingerprints: &std::collections::BTreeSet<PipelineFingerprint>,
    ) -> Result<(), PayloadError> {
        if self.version != REFERENCE_VERSION || self.encoded_size == 0 && !self.codecs.is_empty() {
            return Err(PayloadError::MalformedReference);
        }
        let size = usize::try_from(self.encoded_size).map_err(|_| PayloadError::SizeLimit {
            kind: "stored payload",
        })?;
        if size > limits.batch_bytes() {
            return Err(PayloadError::SizeLimit { kind: "batch" });
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(PayloadError::MalformedReference);
        }
        if !fingerprints
            .iter()
            .any(|fingerprint| self.pipeline_fingerprint == fingerprint.to_hex())
        {
            return Err(PayloadError::PipelineMismatch);
        }
        Ok(())
    }

    pub(crate) fn matches_bytes(&self, bytes: &[u8]) -> bool {
        hex(sha256(bytes)) == self.sha256
    }
}

pub(crate) fn hex(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
