//! Temporal SDK data-converter integration for the always-on payload pipeline.

use futures::future::BoxFuture;
use temporalio_common::{
    data_converters::{
        DataConverter, DefaultFailureConverter, PayloadCodec as TemporalPayloadCodecTrait,
        PayloadConversionError, PayloadConverter, SerializationContextData,
    },
    protos::temporal::api::common::v1::Payload as TemporalPayload,
};

use crate::{Payload, PayloadError, PayloadPipeline};

/// A Temporal SDK payload codec backed by one immutable Agentive payload pipeline.
///
/// Every Temporal payload is externalized by the wrapped pipeline, irrespective
/// of input size or whether an optional transform codec declines it.
#[derive(Debug, Clone)]
pub struct TemporalPayloadCodec {
    pipeline: PayloadPipeline,
}

impl TemporalPayloadCodec {
    /// Creates a codec backed by the supplied immutable pipeline.
    #[must_use]
    pub fn new(pipeline: PayloadPipeline) -> Self {
        Self { pipeline }
    }

    /// Borrows the immutable pipeline shared by client, worker, and codec server.
    #[must_use]
    pub fn pipeline(&self) -> &PayloadPipeline {
        &self.pipeline
    }
}

/// Creates the official Temporal SDK converter using the supplied pipeline.
///
/// Applications must install the returned converter equivalently on each
/// Temporal client and worker that reads or writes the same histories.
#[must_use]
pub fn temporal_data_converter(pipeline: PayloadPipeline) -> DataConverter {
    DataConverter::new(
        PayloadConverter::default(),
        DefaultFailureConverter::default(),
        TemporalPayloadCodec::new(pipeline),
    )
}

impl TemporalPayloadCodecTrait for TemporalPayloadCodec {
    fn encode(
        &self,
        _: &SerializationContextData,
        payloads: Vec<TemporalPayload>,
    ) -> BoxFuture<'static, Result<Vec<TemporalPayload>, PayloadConversionError>> {
        let pipeline = self.pipeline.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                payloads
                    .into_iter()
                    .map(|payload| {
                        pipeline
                            .encode(from_temporal(payload))
                            .map(into_temporal)
                            .map_err(codec_error)
                    })
                    .collect()
            })
            .await
            .map_err(|error| PayloadConversionError::EncodingError(Box::new(error)))?
        })
    }

    fn decode(
        &self,
        _: &SerializationContextData,
        payloads: Vec<TemporalPayload>,
    ) -> BoxFuture<'static, Result<Vec<TemporalPayload>, PayloadConversionError>> {
        let pipeline = self.pipeline.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                payloads
                    .into_iter()
                    .map(|payload| {
                        pipeline
                            .decode(from_temporal(payload))
                            .map(into_temporal)
                            .map_err(codec_error)
                    })
                    .collect()
            })
            .await
            .map_err(|error| PayloadConversionError::EncodingError(Box::new(error)))?
        })
    }
}

fn from_temporal(payload: TemporalPayload) -> Payload {
    Payload::new(payload.data).with_metadata_map(payload.metadata.into_iter().collect())
}

fn into_temporal(payload: Payload) -> TemporalPayload {
    TemporalPayload {
        metadata: payload.metadata().clone().into_iter().collect(),
        data: payload.data().to_vec(),
        external_payloads: Vec::new(),
    }
}

fn codec_error(error: PayloadError) -> PayloadConversionError {
    PayloadConversionError::EncodingError(Box::new(error))
}
