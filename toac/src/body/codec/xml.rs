//! XML codec: `application/xml` and `text/xml` bodies.
//!
//! Backed by [`quick_xml`]'s serde integration, so any payload that
//! satisfies [`serde::Serialize`] / [`serde::de::DeserializeOwned`] is
//! supported without an extra schema description. Like [`super::json`],
//! the codec serialises owned values for encoding and collects the body
//! before deserialising on the response side.
//!
//! Gated behind the `xml` feature on `toac`. Generated code emits the
//! codec only when the spec declares an XML media type and the consumer
//! turned the feature on.

use bytes::Bytes;
use http::HeaderValue;
use http_body_util::{BodyExt, Full};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    BoxError,
    body::{
        Body,
        codec::{BodyContentType, BodyDecoder, BodyEncoder},
    },
};

/// Re-export of [`quick_xml::SeError`] under a name the generator can
/// reach without forcing every generated crate to depend on `quick-xml`
/// directly. Mirrors `serde_json::Error` for the JSON codec.
pub type XmlEncodeError = quick_xml::SeError;

/// XML encoder. `content_type` defaults to `application/xml`; override
/// for `text/xml` or vendor-specific MIMEs that share the same wire
/// shape.
#[derive(Clone, Debug)]
pub struct XmlEncoder {
    pub content_type: HeaderValue,
}

impl Default for XmlEncoder {
    fn default() -> Self {
        Self {
            content_type: HeaderValue::from_static("application/xml"),
        }
    }
}

impl XmlEncoder {
    /// Overrides the wire `Content-Type` — useful for `text/xml` or
    /// vendor-specific XML MIMEs like `application/atom+xml`.
    pub fn with_content_type(content_type: HeaderValue) -> Self {
        Self { content_type }
    }
}

impl BodyContentType for XmlEncoder {
    fn content_type(&self) -> HeaderValue {
        self.content_type.clone()
    }
}

impl<T: Serialize> BodyEncoder<&T> for XmlEncoder {
    type Error = quick_xml::SeError;

    fn encode(&self, data: &T) -> Result<Body, Self::Error> {
        let serialised = quick_xml::se::to_string(data)?;
        Ok(Body::new(Full::new(Bytes::from(serialised.into_bytes()))))
    }
}

/// XML decoder. Collects the streaming body and deserialises it through
/// [`quick_xml::de::from_reader`].
#[derive(Clone, Debug, Default)]
pub struct XmlDecoder;

/// Failure modes for XML decoding.
#[derive(Debug, thiserror::Error)]
pub enum XmlDecodeError {
    /// Collecting the streaming body failed.
    #[error("body read error: {0}")]
    Body(#[source] BoxError),
    /// `quick-xml` rejected the payload.
    #[error("xml decode error: {0}")]
    Xml(#[from] quick_xml::DeError),
}

impl<O> BodyDecoder<O> for XmlDecoder
where
    O: DeserializeOwned,
{
    type Error = XmlDecodeError;

    async fn decode<B>(&self, body: B) -> Result<O, Self::Error>
    where
        B: http_body::Body<Data = Bytes> + Send + Sync + 'static,
        B::Error: Into<BoxError>,
    {
        let bytes = body
            .collect()
            .await
            .map_err(|e| XmlDecodeError::Body(e.into()))?
            .to_bytes();
        // `quick_xml` requires a `&str`; the body must be valid UTF-8
        // for serde-style deserialisation to make sense at all.
        let text = std::str::from_utf8(bytes.as_ref()).map_err(|e| {
            XmlDecodeError::Xml(quick_xml::DeError::Custom(format!(
                "non-UTF-8 XML payload: {e}"
            )))
        })?;
        Ok(quick_xml::de::from_str(text)?)
    }
}
