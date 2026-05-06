//! Tower-compatible OpenAPI client runtime.
//!
//! `toac` is the library half of the code-generation/runtime split: the
//! `toac-build` crate emits Rust code at build time, and the generated
//! code links against the types and traits defined here.
//!
//! The two body-transform traits — [`IntoHttpRequest`] and
//! [`FromHttpResponse`] — plumb generated `{Op}Request` /
//! `{Op}Response` values through any `http::Request` /
//! `http::Response` transport. [`ApiClient`] wraps a
//! [`tower::Service`] and adapts it to `Service<Op>`, so callers
//! drive the API through typed operation values.

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

/// Byte buffer whose textual/serde projection is standard base64.
///
/// OpenAPI's `type: string, format: byte` expects base64-encoded payloads
/// on the wire while the decoded value is raw bytes. `Base64String` keeps
/// bytes in memory (through [`bytes::Bytes`]) and transparently flips to
/// a base64 string whenever the value crosses a serde boundary or is
/// displayed.
///
/// Round-trip: `serde_json::to_string` / `from_str` always goes through
/// base64 — decoding rejects invalid input with a serde error.
#[cfg(feature = "base64")]
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct Base64String(::bytes::Bytes);

#[cfg(feature = "base64")]
impl Base64String {
    /// Wraps raw bytes without encoding.
    pub fn from_bytes(bytes: impl Into<::bytes::Bytes>) -> Self {
        Self(bytes.into())
    }

    /// Returns a view over the raw bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_ref()
    }

    /// Extracts the underlying [`bytes::Bytes`] without copying.
    pub fn into_bytes(self) -> ::bytes::Bytes {
        self.0
    }

    /// Decodes a base64 string using the standard alphabet with padding.
    ///
    /// # Errors
    ///
    /// Returns [`base64::DecodeError`] if the input is not valid base64.
    pub fn decode(encoded: &str) -> Result<Self, ::base64::DecodeError> {
        use ::base64::Engine as _;
        let bytes = ::base64::engine::general_purpose::STANDARD.decode(encoded)?;
        Ok(Self(::bytes::Bytes::from(bytes)))
    }

    /// Encodes the contained bytes as a base64 string.
    pub fn encode(&self) -> String {
        use ::base64::Engine as _;
        ::base64::engine::general_purpose::STANDARD.encode(self.0.as_ref())
    }
}

#[cfg(feature = "base64")]
impl std::fmt::Display for Base64String {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.encode())
    }
}

#[cfg(feature = "base64")]
impl std::fmt::Debug for Base64String {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Base64String").field(&self.encode()).finish()
    }
}

#[cfg(feature = "base64")]
impl From<::bytes::Bytes> for Base64String {
    fn from(value: ::bytes::Bytes) -> Self {
        Self(value)
    }
}

#[cfg(feature = "base64")]
impl From<Vec<u8>> for Base64String {
    fn from(value: Vec<u8>) -> Self {
        Self(::bytes::Bytes::from(value))
    }
}

#[cfg(feature = "base64")]
impl AsRef<[u8]> for Base64String {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

#[cfg(feature = "base64")]
impl serde::Serialize for Base64String {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.encode())
    }
}

#[cfg(feature = "base64")]
impl<'de> serde::Deserialize<'de> for Base64String {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;
        let encoded =
            <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(deserializer)?;
        Self::decode(&encoded).map_err(D::Error::custom)
    }
}

#[cfg(all(test, feature = "base64"))]
mod base64_tests {
    use super::Base64String;

    #[test]
    fn json_round_trip() {
        let original = Base64String::from_bytes(b"hello".as_slice().to_vec());
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"aGVsbG8=\"");
        let decoded: Base64String = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.as_bytes(), b"hello");
    }

    #[test]
    fn display_emits_base64() {
        let v = Base64String::from_bytes(vec![0x00, 0xff, 0x10]);
        assert_eq!(v.to_string(), "AP8Q");
    }

    #[test]
    fn invalid_base64_errors_on_deserialize() {
        let err = serde_json::from_str::<Base64String>("\"not base64!\"").unwrap_err();
        assert!(
            err.to_string().contains("Invalid")
                || err.to_string().to_lowercase().contains("base64")
        );
    }
}
