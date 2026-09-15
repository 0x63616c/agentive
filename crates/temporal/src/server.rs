//! Framework-neutral codec-server request handling.

use std::{
    convert::Infallible,
    future::{Ready, ready},
    task::{Context, Poll},
};

use http::{Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use tower::Service;

use crate::{Payload, PayloadError, PayloadPipeline};

/// Wire request accepted by the embeddable Codec Server Tower service.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodecServerRequest {
    payload: Payload,
}

impl CodecServerRequest {
    /// Creates a decode request from one Temporal-compatible payload.
    #[must_use]
    pub fn new(payload: Payload) -> Self {
        Self { payload }
    }
}

/// Wire response emitted by the embeddable Codec Server Tower service.
#[derive(Debug, Serialize)]
pub struct CodecServerResponse {
    payload: Payload,
}

impl CodecServerResponse {
    /// Returns the decoded payload.
    #[must_use]
    pub fn payload(&self) -> &Payload {
        &self.payload
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
        if request.body().len() > self.pipeline.limits().batch_bytes() {
            return Self::response(
                StatusCode::PAYLOAD_TOO_LARGE,
                serde_json::json!({"error":"payload exceeds configured batch limit"}),
            );
        }
        match serde_json::from_slice::<CodecServerRequest>(request.body())
            .map_err(PayloadError::from)
            .and_then(|request| self.decode_payload(request.payload))
        {
            Ok(payload) => Self::response(StatusCode::OK, CodecServerResponse { payload }),
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
        let limit = server.pipeline.limits().batch_bytes();
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
