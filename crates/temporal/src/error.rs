use std::io;

use thiserror::Error;

/// Whether an owning runtime may safely retry an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDisposition {
    /// Retrying the same operation may succeed.
    Retryable,
    /// Retrying cannot correct the input or integrity failure.
    Terminal,
}

/// A typed payload, codec, or external-storage failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PayloadError {
    /// A public identifier or reference field was not valid.
    #[error("invalid {kind}: {reason}")]
    InvalidInput {
        /// The rejected input category.
        kind: &'static str,
        /// The validation reason.
        reason: &'static str,
    },
    /// An immutable object was not present.
    #[error("external payload object is missing")]
    MissingObject,
    /// Immutable creation encountered differing bytes under an existing ID.
    #[error("immutable object conflicts with different bytes")]
    ImmutableConflict,
    /// Bytes retrieved from storage failed integrity verification.
    #[error("external payload integrity verification failed")]
    IntegrityMismatch,
    /// A configured size bound would be exceeded.
    #[error("payload exceeds configured {kind} limit")]
    SizeLimit {
        /// The bounded resource category.
        kind: &'static str,
    },
    /// A referenced storage identity was not registered.
    #[error("unknown storage id")]
    UnknownStorage,
    /// A reference was produced by a different pipeline configuration.
    #[error("payload pipeline fingerprint does not match")]
    PipelineMismatch,
    /// An encoded transform could not be selected or decoded.
    #[error("codec is unavailable or does not support its recorded version")]
    UnknownCodec,
    /// An encoded reference was structurally invalid.
    #[error("malformed external payload reference")]
    MalformedReference,
    /// A transform failed while encoding or decoding bytes.
    #[error("payload codec failed: {0}")]
    Codec(String),
    /// An I/O failure occurred at the driver boundary.
    #[error("storage I/O failed: {source}")]
    Io {
        /// The underlying filesystem error.
        #[source]
        source: io::Error,
        /// Retry advice classified at the storage boundary.
        retry: RetryDisposition,
    },
    /// JSON input or output was not valid for this protocol.
    #[error("payload JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

impl PayloadError {
    /// Returns retry advice; drivers and codecs never perform hidden retries.
    #[must_use]
    pub const fn retry_disposition(&self) -> RetryDisposition {
        match self {
            Self::Io { retry, .. } => *retry,
            Self::Codec(_) => RetryDisposition::Terminal,
            Self::InvalidInput { .. }
            | Self::MissingObject
            | Self::ImmutableConflict
            | Self::IntegrityMismatch
            | Self::SizeLimit { .. }
            | Self::UnknownStorage
            | Self::PipelineMismatch
            | Self::UnknownCodec
            | Self::MalformedReference
            | Self::Json(_) => RetryDisposition::Terminal,
        }
    }

    pub(crate) fn from_io(source: io::Error) -> Self {
        let retry = match source.kind() {
            io::ErrorKind::Interrupted
            | io::ErrorKind::WouldBlock
            | io::ErrorKind::TimedOut
            | io::ErrorKind::ResourceBusy => RetryDisposition::Retryable,
            _ => RetryDisposition::Terminal,
        };
        Self::Io { source, retry }
    }
}
