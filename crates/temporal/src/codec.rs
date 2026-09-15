use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::PayloadError;

/// A validated stable identity for a payload transform codec.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CodecId(String);

impl CodecId {
    /// Validates a lowercase, dash-separated stable codec identity.
    ///
    /// # Errors
    /// Returns an error when the identity is empty, too long, or contains an
    /// unsupported character.
    pub fn new(value: impl Into<String>) -> Result<Self, PayloadError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if valid {
            Ok(Self(value))
        } else {
            Err(PayloadError::InvalidInput {
                kind: "codec id",
                reason: "must be 1 to 64 lowercase ASCII letters, digits, or dashes",
            })
        }
    }

    #[must_use]
    /// Returns the validated codec identity text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CodecId {
    type Error = PayloadError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<CodecId> for String {
    fn from(value: CodecId) -> Self {
        value.0
    }
}

/// A validated codec wire-format version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CodecVersion(u16);

impl CodecVersion {
    /// Creates a nonzero codec version.
    ///
    /// # Errors
    /// Returns an error for version zero.
    pub const fn new(value: u16) -> Result<Self, PayloadError> {
        if value == 0 {
            Err(PayloadError::InvalidInput {
                kind: "codec version",
                reason: "must be nonzero",
            })
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    /// Returns the nonzero wire-format version.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// A codec's explicit write and read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecDescriptor {
    id: CodecId,
    encode_version: CodecVersion,
    decode_versions: BTreeSet<CodecVersion>,
}

impl CodecDescriptor {
    /// Creates a descriptor with explicit historical decode support.
    ///
    /// # Errors
    /// Returns an error when the writer version is not also readable.
    pub fn new(
        id: CodecId,
        encode_version: CodecVersion,
        decode_versions: impl IntoIterator<Item = CodecVersion>,
    ) -> Result<Self, PayloadError> {
        let decode_versions = decode_versions.into_iter().collect::<BTreeSet<_>>();
        if decode_versions.contains(&encode_version) {
            Ok(Self {
                id,
                encode_version,
                decode_versions,
            })
        } else {
            Err(PayloadError::InvalidInput {
                kind: "codec descriptor",
                reason: "the encode version must be accepted for decoding",
            })
        }
    }

    #[must_use]
    /// Returns this descriptor's stable codec identity.
    pub fn id(&self) -> &CodecId {
        &self.id
    }

    #[must_use]
    /// Returns the version written by this codec.
    pub const fn encode_version(&self) -> CodecVersion {
        self.encode_version
    }

    #[must_use]
    /// Reports whether this descriptor can decode a recorded version.
    pub fn accepts_decode_version(&self, version: CodecVersion) -> bool {
        self.decode_versions.contains(&version)
    }

    #[must_use]
    /// Returns every version accepted for historical decoding.
    pub fn decode_versions(&self) -> &BTreeSet<CodecVersion> {
        &self.decode_versions
    }
}

/// The result of asking one transform to handle bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecOutcome {
    /// The codec transformed the bytes and owns the recorded version.
    Applied(Vec<u8>),
    /// The codec deliberately left the bytes unchanged.
    NotHandled,
}

/// An ordered, reversible byte transformation.
pub trait PayloadCodec: Send + Sync + std::fmt::Debug {
    /// Declares this codec's stable identity and version ownership.
    fn descriptor(&self) -> Result<CodecDescriptor, PayloadError>;

    /// Attempts to transform bytes for storage.
    fn encode(&self, input: &[u8]) -> Result<CodecOutcome, PayloadError>;

    /// Reverses bytes owned by this codec and recorded version.
    fn decode(
        &self,
        version: CodecVersion,
        input: &[u8],
        limit: usize,
    ) -> Result<Vec<u8>, PayloadError>;
}

/// Beneficial Zstandard compression with bounded decompression.
#[derive(Debug, Clone)]
pub struct ZstdCodec {
    level: i32,
}

impl Default for ZstdCodec {
    fn default() -> Self {
        Self { level: 3 }
    }
}

impl ZstdCodec {
    /// Selects a Zstandard compression level.
    #[must_use]
    pub const fn with_level(level: i32) -> Self {
        Self { level }
    }
}

impl PayloadCodec for ZstdCodec {
    fn descriptor(&self) -> Result<CodecDescriptor, PayloadError> {
        CodecDescriptor::new(
            CodecId::new("zstd")?,
            CodecVersion::new(1)?,
            [CodecVersion::new(1)?],
        )
    }

    fn encode(&self, input: &[u8]) -> Result<CodecOutcome, PayloadError> {
        let encoded = zstd::bulk::compress(input, self.level)
            .map_err(|error| PayloadError::Codec(error.to_string()))?;
        if encoded.len() < input.len() {
            Ok(CodecOutcome::Applied(encoded))
        } else {
            Ok(CodecOutcome::NotHandled)
        }
    }

    fn decode(
        &self,
        version: CodecVersion,
        input: &[u8],
        limit: usize,
    ) -> Result<Vec<u8>, PayloadError> {
        if version != CodecVersion::new(1)? {
            return Err(PayloadError::UnknownCodec);
        }
        zstd::bulk::decompress(input, limit).map_err(|error| {
            if error
                .to_string()
                .contains("Destination buffer is too small")
            {
                PayloadError::SizeLimit {
                    kind: "decoded payload",
                }
            } else {
                PayloadError::Codec(error.to_string())
            }
        })
    }
}
