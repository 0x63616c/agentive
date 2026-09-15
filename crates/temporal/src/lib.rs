//! Always-on External Storage payload support for Temporal integrations.
//!
//! This crate deliberately contains no Temporal worker runtime or socket
//! listener. Applications mount [`CodecServer`] behind their own security.

#![forbid(unsafe_code)]

mod codec;
mod error;
mod payload;
mod pipeline;
pub mod runtime;
mod server;
mod storage;
mod temporal_codec;

pub use codec::{CodecDescriptor, CodecId, CodecOutcome, CodecVersion, PayloadCodec, ZstdCodec};
pub use error::{PayloadError, RetryDisposition};
pub use payload::{Payload, PayloadLimits, PipelineFingerprint};
pub use pipeline::{PayloadPipeline, PayloadPipelineBuilder};
pub use server::{CodecServer, CodecServerRequest, CodecServerResponse};
pub use storage::{
    FilesystemStorage, InMemoryStorage, ObjectId, ObjectKey, StorageDriver, StorageId,
};
pub use temporal_codec::{TemporalPayloadCodec, temporal_data_converter};

#[cfg(feature = "axum")]
pub use server::axum_router;
