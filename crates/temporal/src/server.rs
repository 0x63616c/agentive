//! Framework-neutral codec-server request handling.

use std::{
    convert::Infallible,
    future::{Ready, ready},
    task::{Context, Poll},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use http::{Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tower::Service;

use crate::{Payload, PayloadError, PayloadPipeline};

/// Wire request accepted by the embeddable Codec Server Tower service.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodecServerRequest {
    payloads: Vec<WirePayload>,
}

impl CodecServerRequest {
    /// Creates a decode request from one Temporal-compatible payload.
    #[must_use]
    pub fn new(payload: Payload) -> Self {
        Self::from_payloads([payload])
    }

    /// Creates a standard Temporal Codec Server request for a payload batch.
    #[must_use]
    pub fn from_payloads(payloads: impl IntoIterator<Item = Payload>) -> Self {
        Self {
            payloads: payloads.into_iter().map(WirePayload::from).collect(),
        }
    }
}

/// Wire response emitted by the embeddable Codec Server Tower service.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodecServerResponse {
    payloads: Vec<WirePayload>,
}

impl CodecServerResponse {
    /// Returns the decoded payload.
    pub fn into_payloads(self) -> Result<Vec<Payload>, PayloadError> {
        self.payloads.into_iter().map(Payload::try_from).collect()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePayload {
    metadata: BTreeMap<String, String>,
    data: String,
}

impl From<Payload> for WirePayload {
    fn from(payload: Payload) -> Self {
        Self {
            metadata: payload
                .metadata()
                .iter()
                .map(|(key, value)| (key.clone(), STANDARD.encode(value)))
                .collect(),
            data: STANDARD.encode(payload.data()),
        }
    }
}

impl TryFrom<WirePayload> for Payload {
    type Error = PayloadError;

    fn try_from(payload: WirePayload) -> Result<Self, Self::Error> {
        let data = STANDARD
            .decode(payload.data)
            .map_err(|_| PayloadError::InvalidInput {
                kind: "codec server payload",
                reason: "payload data must be canonical base64",
            })?;
        let metadata = payload
            .metadata
            .into_iter()
            .map(|(key, value)| {
                STANDARD
                    .decode(value)
                    .map(|value| (key, value))
                    .map_err(|_| PayloadError::InvalidInput {
                        kind: "codec server payload",
                        reason: "payload metadata must be canonical base64",
                    })
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        Ok(Payload::new(data).with_metadata_map(metadata))
    }
}

/// A framework-neutral HTTP/Tower codec handler with no listener or auth policy.
#[derive(Debug, Clone)]
pub struct CodecServer {
    pipeline: PayloadPipeline,
}

impl CodecServer {
    /// Binds the handler to one immutable payload pipeline.
    #[must_use]
    pub fn new(pipeline: PayloadPipeline) -> Self {
        Self { pipeline }
    }

    /// Decodes one payload through the configured immutable pipeline.
    ///
    /// # Errors
    /// Returns the pipeline's typed storage, integrity, codec, or limit failure.
    pub fn decode_payload(&self, payload: Payload) -> Result<Payload, PayloadError> {
        self.pipeline.decode(payload)
    }

    /// Decodes a standard Temporal payload batch through the configured pipeline.
    pub fn decode_payloads(&self, payloads: Vec<Payload>) -> Result<Vec<Payload>, PayloadError> {
        self.pipeline.decode_batch(payloads)
    }

    fn wire_body_limit(&self) -> usize {
        self.pipeline
            .limits()
            .batch_bytes()
            .saturating_mul(2)
            .saturating_add(64 * 1024)
    }

    pub(crate) fn response(status: StatusCode, value: impl Serialize) -> Response<Vec<u8>> {
        let body = serde_json::to_vec(&value)
            .unwrap_or_else(|_| b"{\"error\":\"response serialization failed\"}".to_vec());
        Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(body)
            .unwrap_or_else(|_| {
                Response::new(b"{\"error\":\"response construction failed\"}".to_vec())
            })
    }

    pub(crate) fn handle(&self, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
        if request.method() != http::Method::POST || request.uri().path() != "/decode" {
            return Self::response(
                StatusCode::NOT_FOUND,
                serde_json::json!({"error":"not found"}),
            );
        }
        if request.body().len() > self.wire_body_limit() {
            return Self::response(
                StatusCode::PAYLOAD_TOO_LARGE,
                serde_json::json!({"error":"payload exceeds configured batch limit"}),
            );
        }
        match serde_json::from_slice::<CodecServerRequest>(request.body())
            .map_err(PayloadError::from)
            .and_then(|request| {
                request
                    .payloads
                    .into_iter()
                    .map(Payload::try_from)
                    .collect::<Result<Vec<_>, _>>()
            })
            .and_then(|payloads| self.decode_payloads(payloads))
        {
            Ok(payloads) => Self::response(
                StatusCode::OK,
                CodecServerResponse {
                    payloads: payloads.into_iter().map(WirePayload::from).collect(),
                },
            ),
            Err(error) => Self::response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({"error":error.to_string(),"retry":format!("{:?}", error.retry_disposition())}),
            ),
        }
    }
}

impl Service<Request<Vec<u8>>> for CodecServer {
    type Response = Response<Vec<u8>>;
    type Error = Infallible;
    type Future = Ready<Result<Self::Response, Self::Error>>;
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn call(&mut self, request: Request<Vec<u8>>) -> Self::Future {
        ready(Ok(self.handle(request)))
    }
}

/// Builds the optional Axum convenience router for the Codec Server.
#[cfg(feature = "axum")]
pub fn axum_router(server: CodecServer) -> axum::Router {
    use axum::{
        Router,
        body::{Body, to_bytes},
        extract::{DefaultBodyLimit, Request as AxumRequest, State},
        routing::any,
    };
    async fn delegate(State(server): State<CodecServer>, request: AxumRequest) -> Response<Body> {
        let limit = server.wire_body_limit();
        let (parts, body) = request.into_parts();
        let bytes = match to_bytes(body, limit).await {
            Ok(bytes) => bytes,
            Err(_) => {
                return CodecServer::response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    serde_json::json!({"error":"payload exceeds configured batch limit"}),
                )
                .map(Body::from);
            }
        };
        let response = match tokio::task::spawn_blocking(move || {
            server.handle(Request::from_parts(parts, bytes.to_vec()))
        })
        .await
        {
            Ok(response) => response,
            Err(_) => CodecServer::response(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({"error":"codec worker unavailable"}),
            ),
        };
        response.map(Body::from)
    }
    Router::new()
        .fallback(any(delegate))
        // The pipeline's validated batch limit, not Axum's generic 2 MiB
        // default, owns admission for this raw codec endpoint.
        .layer(DefaultBodyLimit::disable())
        .with_state(server)
}
