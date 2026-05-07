//! Tower-compatible OpenAPI client runtime.
//!
//! `toac` is the library half of the code-generation/runtime split: the
//! `toac-build` crate emits Rust code at build time, and the generated
//! code links against the types and traits defined here.
//!
//! The runtime pins a single pair of transport types — [`Request`] and
//! [`Response`] — both parameterised over the erased [`body::Body`]
//! defined in this crate. Every generated `{Op}Request` implements
//! [`MakeRequest`] to encode itself into a [`Request`], and every
//! generated `{Op}Response` implements [`ParseResponse`] to decode a
//! [`Response`] into a typed variant. [`ApiClient`] wraps a
//! [`tower::Service`] that speaks `Request → Response` and adapts it to
//! `Service<Op>`, so callers drive the API through typed operation
//! values.
//!
//! Because the body type is fixed, the inner transport just needs to
//! accept a [`Request`] and return a [`Response`]. Adapting an arbitrary
//! HTTP client (hyper, reqwest, etc.) usually means a single
//! `tower::Service` layer that converts between the foreign body and
//! [`body::Body`] — [`body::Body::new`] accepts any
//! `http_body::Body<Data = Bytes>` whose error is convertible into
//! [`BoxError`].

use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

pub mod body;
mod error;
mod request;
mod response;

pub use error::BoxError;
pub use request::Request;
pub use response::Response;

/// Converts a generated request value into a [`Request`].
///
/// Consumption (`self`) is intentional: values like request bodies are
/// moved into the HTTP request without extra cloning. Implementations
/// substitute path template placeholders, append query parameters, set
/// header parameters, and encode the body into [`body::Body`].
pub trait MakeRequest {
    /// Builds the HTTP request ready for the transport layer.
    fn make_request(self) -> impl Future<Output = Request> + Send;
}

/// Decodes a generated response enum from any [`http::Response`] whose
/// body frames carry [`bytes::Bytes`].
///
/// The trait is deliberately not tied to [`Response`] (the runtime's
/// canonical alias over [`body::Body`]): real transports return their
/// own body types (`hyper::body::Incoming`, `reqwest::Body`, …), and the
/// generated code only needs [`http_body::Body::Data`] to be
/// [`bytes::Bytes`] to run the collect-then-dispatch pipeline. The
/// method is generic over `B` so callers never have to insert a body
/// adapter layer just to satisfy a `ParseResponse` impl.
///
/// Collecting the streaming body is the implementor's responsibility —
/// generated code reads the body via [`http_body_util`] before
/// dispatching on status. The trait returns an `impl Future` so the
/// async boundary is explicit and the bound `+ Send` can be spelled out.
pub trait ParseResponse: Sized {
    /// Decoding errors raised when the response does not match any known
    /// variant or when payload parsing fails.
    type Error: std::error::Error;

    /// Consumes the response and dispatches it into a generated variant.
    ///
    /// # Errors
    ///
    /// Implementors return [`Self::Error`] for unknown status codes and
    /// payload decoding failures.
    fn parse_response<B>(
        response: ::http::Response<B>,
    ) -> impl Future<Output = Result<Self, Self::Error>> + Send
    where
        B: http_body::Body<Data = ::bytes::Bytes> + Send + 'static,
        B::Error: Into<BoxError>;
}

/// Shared error type used by every generated [`ParseResponse`] impl.
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
    /// Wraps the erased error reported by [`body::Body`].
    #[error("failed to read response body: {0}")]
    BodyRead(#[source] BoxError),

    /// The response body could not be deserialised into the variant's
    /// payload type.
    #[error("failed to decode response body: {0}")]
    Deserialize(#[from] serde_json::Error),
}

/// Couples a generated request type with its response enum.
///
/// Both sides of the exchange use the fixed [`body::Body`] type, so this
/// trait carries no body-related type parameters — it just links the
/// request-side [`MakeRequest`] impl to the [`ParseResponse`] impl that
/// decodes the matching response.
pub trait Operation: MakeRequest {
    /// The response enum produced by [`ParseResponse::parse_response`]
    /// for this operation's [`Response`].
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
/// Holds an inner service `S` that speaks [`Request`] → [`Response`] and
/// a base URL used to resolve the relative URIs produced by
/// [`MakeRequest`] implementations.
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

impl<S, Op, B> tower::Service<Op> for ApiClient<S>
where
    Op: Operation + Send + 'static,
    Op::Response: ParseResponse + Send + 'static,
    <Op::Response as ParseResponse>::Error: Into<DecodeError> + Send + 'static,
    S: tower::Service<Request, Response = ::http::Response<B>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    B: http_body::Body<Data = ::bytes::Bytes> + Send + 'static,
    B::Error: Into<BoxError>,
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
            let http_req = op.make_request().await;
            let http_req = prefix_base_url(http_req, &base_url);
            let http_resp = inner.call(http_req).await.map_err(CallError::Transport)?;
            Op::Response::parse_response(http_resp)
                .await
                .map_err(|e| CallError::Decode(e.into()))
        })
    }
}

/// Prefixes the request's URI with `base_url` when the URI is relative
/// (path-only). Absolute URIs pass through untouched.
fn prefix_base_url(req: Request, base_url: &str) -> Request {
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

/// Includes a generated client module produced by `toac_build::Builder`.
///
/// Pass the spec's *stem* — i.e. the file name without extension.
/// `Builder::new("openapi.yml")` writes `$OUT_DIR/openapi.rs`, so on
/// the consumer side you pair it with `toac::include_client!("openapi")`.
///
/// # Example
///
/// ```ignore
/// // src/lib.rs
/// toac::include_client!("openapi");
/// ```
///
/// For multiple specs, wrap each call in its own module:
///
/// ```ignore
/// pub mod pets {
///     toac::include_client!("pets");
/// }
/// pub mod users {
///     toac::include_client!("users");
/// }
/// ```
#[macro_export]
macro_rules! include_client {
    ($stem:literal) => {
        include!(concat!(env!("OUT_DIR"), "/", $stem, ".rs"));
    };
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
