//! Runtime support traits and helpers for generated client code.
//!
//! The generated `{Op}Request` and `{Op}Response` types implement the two
//! body-transform traits defined here. [`ApiClient`] then binds any
//! [`tower::Service`] that speaks `http::Request` / `http::Response` into a
//! `Service<Op>` that accepts a typed request and yields the typed
//! response enum.

use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use http::{Request, Response};
use http_body::Body;

/// Converts a generated request value into an [`http::Request`] carrying
/// the encoded body type `B`.
///
/// Consumption (`self`) is intentional: values like request bodies are
/// moved into the HTTP request without extra cloning.
pub trait IntoHttpRequest<B: Body> {
    /// Builds the HTTP request, substituting path template placeholders,
    /// appending query parameters, setting header parameters, and
    /// encoding the body.
    fn into_http_request(self) -> impl Future<Output = http::Request<B>> + Send;
}

/// Decodes a generated response enum from an [`http::Response`] whose body
/// has already been collected into `B` (typically [`bytes::Bytes`]).
///
/// Body collection is asynchronous and therefore left to the caller; the
/// trait itself is synchronous so it can be used in non-async contexts.
pub trait FromHttpResponse<B: Body>: Sized {
    /// Decoding errors raised when the response does not match any known
    /// variant or when payload parsing fails.
    type Error: std::error::Error;

    /// Consumes the response and dispatches it into a generated variant.
    ///
    /// # Errors
    ///
    /// Implementors return [`Self::Error`] for unknown status codes and
    /// payload decoding failures.
    fn from_http_response(
        response: http::Response<B>,
    ) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

/// Shared error type used by every generated [`FromHttpResponse`] impl.
///
/// Kept simple on purpose: a mismatch on status is distinguished from a
/// payload parse failure, and the rest bubbles up through the usual
/// [`std::error::Error`] plumbing.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The response's status code matches none of the statuses declared
    /// in the OpenAPI operation.
    #[error("unexpected HTTP status: {0}")]
    UnexpectedStatus(http::StatusCode),

    /// Collecting the streaming response body failed.
    ///
    /// The underlying error comes from the [`http_body::Body`]
    /// implementation the caller provided.
    #[error("failed to read response body: {0}")]
    BodyRead(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// The response body could not be deserialised into the variant's
    /// payload type.
    #[error("failed to decode response body: {0}")]
    Deserialize(#[from] serde_json::Error),
}

/// Couples a generated request type with its response enum.
///
/// The request body type is pinned (each `{Op}Request` serialises into
/// exactly one body shape), while the response body is left flexible:
/// the response enum implements [`FromHttpResponse`] generically, so the
/// same operation can drive any transport whose `tower::Service`
/// produces a `Body` implementation.
pub trait Operation: IntoHttpRequest<Self::RequestBody> {
    /// The `http::Request` body type produced by
    /// [`IntoHttpRequest::into_http_request`].
    type RequestBody: Body + Send;

    /// The response enum decoded from the raw HTTP response. It must
    /// implement [`FromHttpResponse<RespBody>`] for the concrete
    /// response-body type used by the transport, but that `RespBody` is
    /// chosen on the call site by the [`ApiClient`]'s inner service.
    type Response;
}

/// Errors raised by [`ApiClient`]'s `Service::call`.
///
/// Keeps transport failures distinct from payload decoding failures so
/// callers can act on them without string-matching.
#[derive(Debug, thiserror::Error)]
pub enum CallError<TransportError> {
    /// The underlying [`tower::Service`] returned an error while running
    /// the request.
    #[error("transport error: {0}")]
    Transport(#[source] TransportError),

    /// The response was received but could not be decoded into the
    /// operation's response enum.
    #[error("decode error: {0}")]
    Decode(#[source] DecodeError),
}

/// Tower service that turns typed operation requests into HTTP exchanges.
///
/// Holds an inner service `S` that speaks `http::Request<ReqBody>` →
/// `http::Response<RespBody>` and a base URL used to resolve the relative
/// URIs produced by [`IntoHttpRequest`] implementations.
///
/// The client itself is `Clone` so it can be consumed by middleware like
/// [`tower::ServiceExt::ready`] — provided the inner service is `Clone`.
#[derive(Debug, Clone)]
pub struct ApiClient<S> {
    inner: S,
    base_url: Arc<str>,
}

impl<S> ApiClient<S> {
    /// Wraps `inner` with a base URL used to prefix every relative
    /// request URI. `base_url` is stored as-is; no trailing-slash
    /// normalisation is performed — the generator's path templates
    /// always start with `/`.
    pub fn new(inner: S, base_url: impl Into<Arc<str>>) -> Self {
        Self {
            inner,
            base_url: base_url.into(),
        }
    }

    /// Returns a reference to the base URL used by this client.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Consumes the client and returns the inner service.
    pub fn into_inner(self) -> S {
        self.inner
    }

    /// Returns a mutable reference to the inner service.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }
}

impl<S, Op, RespBody> tower::Service<Op> for ApiClient<S>
where
    Op: Operation + Send + 'static,
    Op::RequestBody: Send + 'static,
    Op::Response: FromHttpResponse<RespBody> + Send + 'static,
    <Op::Response as FromHttpResponse<RespBody>>::Error: Into<DecodeError> + Send + 'static,
    RespBody: Body + Send + 'static,
    S: tower::Service<Request<Op::RequestBody>, Response = Response<RespBody>>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = Op::Response;
    type Error = CallError<S::Error>;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(CallError::Transport)
    }

    fn call(&mut self, op: Op) -> Self::Future {
        // Tower pattern: the now-ready inner service goes into the
        // future; a fresh clone takes its place to service the next
        // `poll_ready`.
        let inner = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, inner);
        let base_url = self.base_url.clone();
        Box::pin(async move {
            let http_req = op.into_http_request().await;
            let http_req = prefix_base_url(http_req, &base_url);
            let http_resp = inner.call(http_req).await.map_err(CallError::Transport)?;
            Op::Response::from_http_response(http_resp)
                .await
                .map_err(|e| CallError::Decode(e.into()))
        })
    }
}

/// Prefixes the request's URI with `base_url` when the URI is relative
/// (path-only). Absolute URIs pass through untouched.
fn prefix_base_url<B>(req: Request<B>, base_url: &str) -> Request<B> {
    let (mut parts, body) = req.into_parts();
    let uri = parts.uri.clone();
    if uri.scheme().is_some() {
        return Request::from_parts(parts, body);
    }
    let path_and_query = uri
        .path_and_query()
        .map(ToString::to_string)
        .unwrap_or_default();
    let combined = format!("{}{}", trim_trailing_slash(base_url), path_and_query);
    if let Ok(new_uri) = combined.parse() {
        parts.uri = new_uri;
    }
    Request::from_parts(parts, body)
}

/// Strips one trailing `/` so combining with a path that starts with `/`
/// doesn't produce a doubled separator.
fn trim_trailing_slash(s: &str) -> &str {
    s.strip_suffix('/').unwrap_or(s)
}
