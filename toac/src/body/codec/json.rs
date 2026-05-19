use crate::error::BoxError;
use bytes::{Buf, Bytes};
use http::HeaderValue;
use http_body_util::{BodyExt, Full};
use serde::de::DeserializeOwned;

use crate::body::codec::{BodyContentType, BodyDecoder, BodyEncoder};

#[derive(Clone, Debug)]
pub struct JsonEncoder {
    pub pretty: bool,
    pub content_type: HeaderValue,
}

impl Default for JsonEncoder {
    fn default() -> Self {
        Self {
            pretty: false,
            content_type: HeaderValue::from_static("application/json"),
        }
    }
}

impl BodyContentType for JsonEncoder {
    fn content_type(&self) -> HeaderValue {
        self.content_type.clone()
    }
}

impl<T: serde::Serialize> BodyEncoder<&T> for JsonEncoder {
    type Error = serde_json::Error;
    fn encode(&self, data: &T) -> Result<crate::body::Body, Self::Error> {
        let encoded = if self.pretty {
            serde_json::ser::to_vec_pretty(data)
        } else {
            serde_json::ser::to_vec(data)
        }?;
        Ok(crate::body::Body::new(Full::new(encoded.into())))
    }
}
#[derive(Clone, Debug, Default)]
pub struct JsonDecoder;

#[derive(Debug, thiserror::Error)]
pub enum JsonDecodeError {
    #[error("Body Error")]
    Body(#[source] BoxError),
    #[error("Json Decode Error")]
    Json(#[source] serde_json::Error),
}
impl<O> BodyDecoder<O> for JsonDecoder
where
    O: DeserializeOwned,
{
    type Error = JsonDecodeError;
    async fn decode<B>(&self, body: B) -> Result<O, Self::Error>
    where
        B: http_body::Body<Data = Bytes> + Send + Sync + 'static,
        B::Error: Into<BoxError>,
    {
        let reader = body
            .collect()
            .await
            .map_err(|e| JsonDecodeError::Body(e.into()))?
            .aggregate()
            .reader();
        serde_json::de::from_reader(reader).map_err(JsonDecodeError::Json)
    }
}
