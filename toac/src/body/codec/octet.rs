//! Binary codec: `application/octet-stream`, `*/*`, and anything else
//! where the payload is a plain byte buffer.
//!
//! Payload type is [`Bytes`] on both sides — it's cheap to clone and
//! the runtime's existing body plumbing already moves bytes around as
//! [`bytes::Bytes`]. Encoding moves the buffer in without copying;
//! decoding collects the streaming body and hands the bytes back.

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

/// Binary encoder. `content_type` defaults to
/// `application/octet-stream`; override for MIMEs like `image/png`,
/// `application/pdf`, `audio/mpeg` that share the raw-bytes wire
/// shape.
#[derive(Clone, Debug)]
pub struct OctetEncoder {
    pub content_type: HeaderValue,
}

impl Default for OctetEncoder {
    fn default() -> Self {
        Self {
            content_type: HeaderValue::from_static("application/octet-stream"),
        }
    }
}

impl OctetEncoder {
    /// Overrides the wire `Content-Type`. Pass e.g.
    /// `HeaderValue::from_static("image/png")` for a PNG upload.
    pub fn with_content_type(content_type: HeaderValue) -> Self {
        Self { content_type }
    }
}

impl BodyContentType for OctetEncoder {
    fn content_type(&self) -> HeaderValue {
        self.content_type.clone()
    }
}

impl BodyEncoder<Bytes> for OctetEncoder {
    type Error = std::convert::Infallible;

    fn encode(&self, data: Bytes) -> Result<Body, Self::Error> {
        Ok(Body::new(Full::new(data)))
    }
}

impl BodyEncoder<&Bytes> for OctetEncoder {
    type Error = std::convert::Infallible;

    fn encode(&self, data: &Bytes) -> Result<Body, Self::Error> {
        <Self as BodyEncoder<Bytes>>::encode(self, data.clone())
    }
}

impl BodyEncoder<Vec<u8>> for OctetEncoder {
    type Error = std::convert::Infallible;

    fn encode(&self, data: Vec<u8>) -> Result<Body, Self::Error> {
        Ok(Body::new(Full::new(Bytes::from(data))))
    }
}

/// Binary decoder. Produces the collected payload as [`Bytes`].
#[derive(Clone, Debug, Default)]
pub struct OctetDecoder;

/// Failure modes for binary decoding.
#[derive(Debug, thiserror::Error)]
pub enum OctetDecodeError {
    /// Collecting the streaming body failed.
    #[error("body read error: {0}")]
    Body(#[source] BoxError),
}

impl BodyDecoder<Bytes> for OctetDecoder {
    type Error = OctetDecodeError;

    async fn decode<B>(&self, body: B) -> Result<Bytes, Self::Error>
    where
        B: http_body::Body<Data = Bytes> + Send + 'static,
        B::Error: Into<BoxError>,
    {
        let collected = body
            .collect()
            .await
            .map_err(|e| OctetDecodeError::Body(e.into()))?;
        Ok(collected.to_bytes())
    }
}
