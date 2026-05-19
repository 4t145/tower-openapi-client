//! `reqwest` backend adapter.
//!
//! Wraps a [`reqwest::Client`] in a [`tower::Service`] that speaks the
//! runtime's [`crate::Request`] / [`http::Response<reqwest::Body>`] pair,
//! so [`crate::ApiClient`] can drive `reqwest` without any code-gen
//! changes. The response body is `reqwest::Body`, which already satisfies
//! the bound [`crate::ParseResponse`] places on `B`
//! (`http_body::Body<Data = Bytes> + Send + Sync + 'static`).
//!
//! Streaming pass-through: request bodies wrap [`crate::body::Body`] in
//! [`reqwest::Body::wrap`] without an intermediate copy.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use crate::{ApiClient, Request, Server, body::Body};

/// `tower::Service` adapter over a [`reqwest::Client`].
///
/// Cheap to clone; cloning shares the underlying HTTP connection pool.
#[derive(Debug, Clone)]
pub struct ReqwestService {
    client: reqwest::Client,
}

impl ReqwestService {
    /// Wraps an existing client.
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// Returns a reference to the underlying client.
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }
}

impl Default for ReqwestService {
    fn default() -> Self {
        Self::new(reqwest::Client::new())
    }
}

impl From<reqwest::Client> for ReqwestService {
    fn from(client: reqwest::Client) -> Self {
        Self::new(client)
    }
}

/// Errors raised when running a request through [`ReqwestService`].
#[derive(Debug, thiserror::Error)]
pub enum ReqwestError {
    /// The runtime [`Request`] could not be turned into a
    /// [`reqwest::Request`] — typically because the URI cannot be parsed
    /// as an absolute URL.
    #[error("invalid request: {0}")]
    InvalidRequest(#[source] crate::BoxError),

    /// `reqwest` reported a transport-level failure while running the
    /// request (DNS, connect, TLS, IO, decode, …).
    #[error("reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
}

impl tower::Service<Request> for ReqwestService {
    type Response = ::http::Response<reqwest::Body>;
    type Error = ReqwestError;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // `reqwest::Client` has no per-call readiness gate — connection
        // pooling and pacing are internal — so the service is always
        // ready.
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let client = self.client.clone();
        Box::pin(async move {
            let reqwest_req = build_reqwest_request(req)?;
            let resp = client.execute(reqwest_req).await?;
            Ok(into_http_response(resp))
        })
    }
}

/// Converts a runtime [`Request`] into a [`reqwest::Request`].
///
/// # Errors
///
/// Returns [`ReqwestError::InvalidRequest`] when the URI cannot be
/// parsed as an absolute URL (the `ApiClient` layer is expected to have
/// prefixed the base URL beforehand) or when `reqwest` rejects the
/// resulting request shape.
fn build_reqwest_request(req: Request) -> Result<reqwest::Request, ReqwestError> {
    let (parts, body) = req.into_parts();
    let http_req = ::http::Request::from_parts(parts, reqwest::Body::wrap(body));
    reqwest::Request::try_from(http_req).map_err(|e| ReqwestError::InvalidRequest(Box::new(e)))
}

/// Repackages a [`reqwest::Response`] into [`http::Response<reqwest::Body>`].
///
/// `reqwest::Response::into_body` is not stable across all version
/// ranges, so the conversion is spelled out: copy status / version /
/// headers / extensions out, then wrap the rest as the body.
fn into_http_response(resp: reqwest::Response) -> ::http::Response<reqwest::Body> {
    let status = resp.status();
    let version = resp.version();
    let mut builder = ::http::Response::builder().status(status).version(version);
    if let Some(headers) = builder.headers_mut() {
        *headers = resp.headers().clone();
    }
    if let Some(extensions) = builder.extensions_mut() {
        *extensions = resp.extensions().clone();
    }
    builder
        .body(reqwest::Body::from(resp))
        .expect("status/version/headers copied from a valid reqwest response")
}

/// Wraps the request body so [`reqwest::Body::wrap`] accepts it.
///
/// `reqwest::Body::wrap` requires `http_body::Body<Data = Bytes> + Send +
/// Sync + 'static`; [`Body`] satisfies that with the `Sync` boxing in
/// [`crate::body`].
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync + 'static>() {}
    assert_send_sync::<Body>();
};

impl ApiClient<ReqwestService> {
    /// Builds an [`ApiClient`] backed by a [`reqwest::Client`].
    ///
    /// Convenience over `ApiClient::new(ReqwestService::new(client),
    /// server)` for the common case where the user already has a
    /// configured `reqwest::Client`. Equivalent to
    /// `ApiClient::new(ReqwestService::from(client), server)` and works
    /// with any [`Server`] implementor — bare URLs and generated
    /// `ServerOption*` values alike.
    pub fn new_reqwest<Srv: Server>(client: reqwest::Client, server: Srv) -> Self {
        ApiClient::new(ReqwestService::new(client), server)
    }
}
