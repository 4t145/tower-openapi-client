//! `application/x-www-form-urlencoded` codec. Request-body only —
//! response-side form payloads don't exist in practice (OAuth2 token
//! endpoints, the biggest consumer of form requests, answer with JSON).
//!
//! Serialisation piggybacks on [`serde_urlencoded`]. Any `Serialize`
//! value that flattens to a map of string-keyed scalars works; nested
//! structs or non-string keys will fail at encode time the same way
//! they do in the upstream crate.

use bytes::Bytes;
use http::HeaderValue;
use http_body_util::Full;

use crate::body::{
    Body,
    codec::{BodyContentType, BodyEncoder},
};

/// Form-urlencoded encoder. `content_type` defaults to
/// `application/x-www-form-urlencoded`; overridable for completeness
/// even though the MIME rarely varies in practice.
#[derive(Clone, Debug)]
pub struct FormEncoder {
    pub content_type: HeaderValue,
}

impl Default for FormEncoder {
    fn default() -> Self {
        Self {
            content_type: HeaderValue::from_static("application/x-www-form-urlencoded"),
        }
    }
}

impl BodyContentType for FormEncoder {
    fn content_type(&self) -> HeaderValue {
        self.content_type.clone()
    }
}

impl<T: serde::Serialize + ?Sized> BodyEncoder<&T> for FormEncoder {
    type Error = serde_urlencoded::ser::Error;

    fn encode(&self, data: &T) -> Result<Body, Self::Error> {
        let encoded = serde_urlencoded::to_string(data)?;
        Ok(Body::new(Full::new(Bytes::from(encoded))))
    }
}
