//! Body encode / decode traits and the helper functions generated code
//! calls into.
//!
//! Both traits are implemented **on the codec** (not on the payload
//! type) so that a single codec value can drive many operation types,
//! and so configuration like `JsonEncoder::pretty` lives on one
//! reusable instance.

use bytes::Bytes;
use http::HeaderValue;

use crate::{BoxError, Request, body::Body};

pub mod form;
pub mod json;
pub mod multipart;
pub mod octet;
pub mod text;

/// Encodes `data` into `request`'s body and sets its `Content-Type`.
///
/// The caller is responsible for building `request` with the correct
/// method, URI, and any header parameters; this helper only touches the
/// body and the `Content-Type` header. Any prior body on `request` is
/// discarded.
///
/// # Errors
///
/// Propagates the encoder's serialisation error.
pub fn encode_body<E, T>(encoder: &E, data: T, request: Request) -> Result<Request, E::Error>
where
    E: BodyEncoder<T>,
{
    let (mut parts, _) = request.into_parts();
    let body = encoder.encode(data)?;
    parts.headers.insert(
        http::header::CONTENT_TYPE,
        BodyContentType::content_type(encoder),
    );
    Ok(Request::from_parts(parts, body))
}

/// The wire `Content-Type` an encoder advertises.
///
/// Lives on a separate trait from [`BodyEncoder`] because it doesn't
/// depend on the payload type. Splitting it lets one encoder type
/// implement [`BodyEncoder<&T>`] for many different `T`s without
/// having to disambiguate `enc.content_type()` calls.
pub trait BodyContentType {
    /// `Content-Type` header this encoder writes onto the request.
    fn content_type(&self) -> HeaderValue;
}

/// Synchronous body encoder.
///
/// Body encoding is CPU work, not I/O, so this trait is intentionally
/// not `async`. Use [`encode_body`] from generated code to fuse the
/// resulting body back into an existing [`Request`].
pub trait BodyEncoder<T>: BodyContentType {
    /// Serialisation error raised by [`BodyEncoder::encode`].
    type Error: std::error::Error + Send + Sync + 'static;

    /// Serialises `data` into an erased body.
    ///
    /// # Errors
    ///
    /// Returns [`Self::Error`] when the payload cannot be serialised.
    fn encode(&self, data: T) -> Result<Body, Self::Error>;
}

/// Decodes `body` into a typed value `O` using `decoder`.
///
/// # Errors
///
/// Propagates the decoder's error, which usually wraps a body-read
/// failure or a payload deserialisation error.
pub async fn decode_body<D, B, O>(decoder: &D, body: B) -> Result<O, D::Error>
where
    D: BodyDecoder<O>,
    B: http_body::Body<Data = Bytes> + Send + Sync + 'static,
    B::Error: Into<BoxError>,
{
    decoder.decode(body).await
}

/// Asynchronous body decoder.
///
/// Async because decoding has to collect a streaming [`http_body::Body`]
/// before handing bytes to the underlying deserialiser. The returned
/// future is `+ Send` so generated [`crate::ParseResponse`] impls —
/// which themselves return `impl Future + Send` — can forward it
/// directly.
pub trait BodyDecoder<O> {
    /// Decoding error raised by [`BodyDecoder::decode`].
    type Error: std::error::Error + Send + Sync + 'static;

    /// Collects `body` and turns it into an `O`.
    ///
    /// # Errors
    ///
    /// Returns [`Self::Error`] on body-read failure or payload decode
    /// failure.
    fn decode<B>(&self, body: B) -> impl Future<Output = Result<O, Self::Error>> + Send
    where
        B: http_body::Body<Data = Bytes> + Send + Sync + 'static,
        B::Error: Into<BoxError>;
}
