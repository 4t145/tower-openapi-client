//! Newline-delimited JSON codec: `application/x-ndjson` and
//! `application/jsonl` response bodies.
//!
//! Decode-only — OAS specs don't drive request bodies through these
//! MIMEs. The decoded value is itself a [`Stream`] so callers see each
//! event as soon as the transport hands the line over, without
//! buffering the entire response.
//!
//! The codec collapses the inbound generic body into the runtime's
//! [`Body`] type, which is `Unpin`, so the line-buffering state machine
//! over [`http_body_util::BodyDataStream`] composes without an extra
//! `Box<dyn Stream>` indirection.
//!
//! Gated behind the `ndjson` feature on `toac`.
//!
//! [`Stream`]: futures_util::stream::Stream

use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Bytes, BytesMut};
use futures_util::stream::{Stream, StreamExt};
use http_body_util::BodyDataStream;
use serde::de::DeserializeOwned;

use crate::{
    BoxError,
    body::{Body, codec::BodyDecoder},
};

/// Newline-delimited JSON decoder.
///
/// Used as a [`BodyDecoder`] whose decoded value is an [`NdjsonStream`]
/// — i.e. the future returned by [`BodyDecoder::decode`] resolves
/// immediately and yields the lazy stream that pulls lines off the
/// transport.
#[derive(Clone, Debug, Default)]
pub struct NdjsonDecoder;

/// Failure modes for ndjson decoding.
#[derive(Debug, thiserror::Error)]
pub enum NdjsonDecodeError {
    /// The transport reported an error while reading the body.
    #[error("body read error: {0}")]
    Body(#[source] BoxError),
    /// A line failed to deserialise as JSON.
    #[error("ndjson decode error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Streaming ndjson reader. Pulls bytes off the runtime [`Body`],
/// buffers partial lines across frames, and deserialises each complete
/// line into `O`.
pub struct NdjsonStream<O> {
    body: BodyDataStream<Body>,
    buf: BytesMut,
    eof: bool,
    _marker: PhantomData<fn() -> O>,
}

impl<O> NdjsonStream<O>
where
    O: DeserializeOwned + Send + 'static,
{
    /// Wraps an `http_body::Body` into an ndjson stream. The body is
    /// collapsed into the runtime [`Body`] alias so the resulting stream
    /// type stays concrete.
    pub fn new<B>(body: B) -> Self
    where
        B: http_body::Body<Data = Bytes> + Send + Sync + 'static,
        B::Error: Into<BoxError>,
    {
        Self {
            body: BodyDataStream::new(Body::new(body)),
            buf: BytesMut::new(),
            eof: false,
            _marker: PhantomData,
        }
    }
}

impl<O> Stream for NdjsonStream<O>
where
    O: DeserializeOwned,
{
    type Item = Result<O, NdjsonDecodeError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // `Body` is `Unpin`, so `BodyDataStream<Body>` is `Unpin` too —
        // standard borrowing without manual pin-projection works.
        let this = &mut *self;
        loop {
            if let Some(pos) = this.buf.iter().position(|b| *b == b'\n') {
                let line = this.buf.split_to(pos + 1);
                let trimmed = trim_end_newline(&line);
                if trimmed.is_empty() {
                    continue;
                }
                return Poll::Ready(Some(parse_line::<O>(trimmed)));
            }
            if this.eof {
                if this.buf.is_empty() {
                    return Poll::Ready(None);
                }
                let trailing = std::mem::take(&mut this.buf);
                let trimmed = trim_end_newline(&trailing);
                if trimmed.is_empty() {
                    return Poll::Ready(None);
                }
                return Poll::Ready(Some(parse_line::<O>(trimmed)));
            }
            match this.body.poll_next_unpin(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(chunk))) => this.buf.extend_from_slice(chunk.as_ref()),
                Poll::Ready(Some(Err(e))) => {
                    this.eof = true;
                    return Poll::Ready(Some(Err(NdjsonDecodeError::Body(e))));
                }
                Poll::Ready(None) => this.eof = true,
            }
        }
    }
}

fn parse_line<O: DeserializeOwned>(bytes: &[u8]) -> Result<O, NdjsonDecodeError> {
    serde_json::from_slice(bytes).map_err(NdjsonDecodeError::Json)
}

fn trim_end_newline(bytes: &[u8]) -> &[u8] {
    let trimmed = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    trimmed.strip_suffix(b"\r").unwrap_or(trimmed)
}

impl<O> BodyDecoder<NdjsonStream<O>> for NdjsonDecoder
where
    O: DeserializeOwned + Send + 'static,
{
    type Error = std::convert::Infallible;

    async fn decode<B>(&self, body: B) -> Result<NdjsonStream<O>, Self::Error>
    where
        B: http_body::Body<Data = Bytes> + Send + Sync + 'static,
        B::Error: Into<BoxError>,
    {
        Ok(NdjsonStream::new(body))
    }
}
