//! Server-Sent Events codec: `text/event-stream` response bodies.
//!
//! Decode-only — SSE is a response-side wire format, OAS specs don't
//! drive request bodies through it. The decoded value is itself a
//! [`Stream`] over [`sse_stream::Sse`] events; callers pull events as
//! they arrive without buffering the whole stream.
//!
//! Backed by the [`sse-stream`](https://crates.io/crates/sse-stream)
//! crate. The codec collapses the inbound generic body into the
//! runtime's [`Body`] type so the public stream alias stays concrete —
//! no extra `Box<dyn Stream>` indirection.
//!
//! Re-exports [`Sse`] and the underlying error type so generated code
//! never has to spell out the dependency directly.
//!
//! Gated behind the `sse` feature on `toac`.
//!
//! [`Stream`]: futures_util::stream::Stream

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;

use crate::{
    BoxError,
    body::{Body, codec::BodyDecoder},
};

pub use sse_stream::{Error as SseError, Sse};

/// Concrete SSE event stream alias. Itself a
/// [`futures_util::stream::Stream`] of `Result<Sse, SseError>`, so
/// callers use the standard `.next()` / `.collect()` ergonomics without
/// boxing.
pub type SseEventStream = sse_stream::SseStream<SseBody>;

/// Adapter wrapping the runtime [`Body`] so its error type satisfies
/// `sse_stream::SseStream`'s `Error: std::error::Error + Send + Sync +
/// 'static + Sized` bound. [`Body`]'s associated `Error = BoxError`
/// is `Box<dyn Error + Send + Sync>`, which doesn't itself implement
/// [`std::error::Error`] (the blanket impl on `Box<E>` requires `E:
/// Sized`). The newtype below carries the box and provides a sized
/// `std::error::Error` impl that delegates to it.
pub struct SseBody {
    inner: Body,
}

impl SseBody {
    fn new(inner: Body) -> Self {
        Self { inner }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SseBodyError(#[source] BoxError);

impl http_body::Body for SseBody {
    type Data = Bytes;
    type Error = SseBodyError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        // SAFETY: standard pin-projection through a single field.
        let inner = unsafe { self.map_unchecked_mut(|s| &mut s.inner) };
        match inner.poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => Poll::Ready(Some(Ok(frame))),
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(SseBodyError(e)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// SSE decoder. Used as a [`BodyDecoder`] whose decoded value is an
/// [`SseEventStream`].
#[derive(Clone, Debug, Default)]
pub struct SseDecoder;

impl BodyDecoder<SseEventStream> for SseDecoder {
    type Error = std::convert::Infallible;

    async fn decode<B>(&self, body: B) -> Result<SseEventStream, Self::Error>
    where
        B: http_body::Body<Data = Bytes> + Send + Sync + 'static,
        B::Error: Into<BoxError>,
    {
        // Collapse the inbound generic body into the runtime `Body`,
        // then wrap it so the per-frame error meets the `Sized + Error`
        // bound `sse_stream::SseStream` expects.
        Ok(sse_stream::SseStream::new(SseBody::new(Body::new(body))))
    }
}
