//! Plain-text codec: `text/plain` bodies. Payload is a [`String`].
//!
//! Encoding just copies the string's bytes into the body; decoding
//! collects the stream and checks that the bytes are valid UTF-8 —
//! `text/plain` on the wire doesn't carry a charset parameter in OAS
//! specs, so we pin to UTF-8 to avoid an extra config knob. Callers
//! that need other charsets should build their own codec on top of
//! the raw [`super::octet::OctetEncoder`] / [`super::octet::OctetDecoder`].

use bytes::Bytes;
use http::HeaderValue;
use http_body_util::{BodyExt, Full};

use crate::{
    BoxError,
    body::{
        Body,
        codec::{BodyContentType, BodyDecoder, BodyEncoder},
    },
};

/// Text encoder. `content_type` defaults to `text/plain; charset=utf-8`
/// and can be overridden for MIMEs that share the "just bytes of UTF-8"
/// wire shape (e.g. `text/markdown`, `text/html`).
#[derive(Clone, Debug)]
pub struct TextEncoder {
    pub content_type: HeaderValue,
}

impl Default for TextEncoder {
    fn default() -> Self {
        Self {
            content_type: HeaderValue::from_static("text/plain; charset=utf-8"),
        }
    }
}

impl TextEncoder {
    /// Overrides the wire `Content-Type` — useful for `text/markdown`
    /// and other text sub-types.
    pub fn with_content_type(content_type: HeaderValue) -> Self {
        Self { content_type }
    }
}

impl BodyContentType for TextEncoder {
    fn content_type(&self) -> HeaderValue {
        self.content_type.clone()
    }
}

impl BodyEncoder<&str> for TextEncoder {
    type Error = std::convert::Infallible;

    fn encode(&self, data: &str) -> Result<Body, Self::Error> {
        Ok(Body::new(Full::new(Bytes::copy_from_slice(
            data.as_bytes(),
        ))))
    }
}

impl BodyEncoder<&String> for TextEncoder {
    type Error = std::convert::Infallible;

    fn encode(&self, data: &String) -> Result<Body, Self::Error> {
        <Self as BodyEncoder<&str>>::encode(self, data.as_str())
    }
}

/// Text decoder. Produces a [`String`] after UTF-8 validation.
#[derive(Clone, Debug, Default)]
pub struct TextDecoder;

/// Failure modes for text decoding.
#[derive(Debug, thiserror::Error)]
pub enum TextDecodeError {
    /// Collecting the streaming body failed.
    #[error("body read error: {0}")]
    Body(#[source] BoxError),
    /// The bytes were not valid UTF-8.
    #[error("utf-8 decode error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

impl BodyDecoder<String> for TextDecoder {
    type Error = TextDecodeError;

    async fn decode<B>(&self, body: B) -> Result<String, Self::Error>
    where
        B: http_body::Body<Data = Bytes> + Send + 'static,
        B::Error: Into<BoxError>,
    {
        let bytes = body
            .collect()
            .await
            .map_err(|e| TextDecodeError::Body(e.into()))?
            .to_bytes();
        String::from_utf8(bytes.to_vec()).map_err(TextDecodeError::Utf8)
    }
}
