//! Erased body type used across the runtime surface.
//!
//! Both [`crate::Request`] and [`crate::Response`] carry a [`Body`]
//! regardless of the underlying transport. This keeps the generated
//! code monomorphic — every `MakeRequest` impl produces the same
//! `Request` type and every `ParseResponse` impl consumes the same
//! `Response` — and lets callers adapt any `http_body::Body<Data =
//! Bytes>` (from hyper, reqwest, etc.) into the runtime with a single
//! `Body::new` call.

pub mod codec;

use std::pin::Pin;
use std::task::Poll;

use http_body_util::{BodyExt, combinators::BoxBody as InnerBoxBody};

/// Internal boxed body; errors are erased into [`crate::BoxError`].
///
/// `Sync` is required because mainstream HTTP backends
/// (`reqwest::Body`, `axum::body::Body`, the bodies plumbed by
/// `tower-http` middleware) all want `Send + Sync`. Picking the `Sync`
/// variant up front means generated code and adapter layers never have
/// to thread `!Sync` through Tower middleware.
type BoxBody = InnerBoxBody<::bytes::Bytes, crate::BoxError>;

/// Unified request / response body.
///
/// Represents either an empty body or an erased streaming body whose
/// frames carry [`bytes::Bytes`]. Anything that implements
/// `http_body::Body<Data = bytes::Bytes> + Send + Sync` with an error
/// convertible into [`crate::BoxError`] can be wrapped through
/// [`Body::new`].
#[derive(Debug)]
pub struct Body {
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    Empty,
    Wrap(BoxBody),
}

impl Body {
    fn from_kind(kind: Kind) -> Self {
        Self { kind }
    }

    /// Creates an empty body. `size_hint` reports zero bytes and
    /// `is_end_stream` returns `true` immediately.
    pub const fn empty() -> Self {
        Self { kind: Kind::Empty }
    }

    /// Wraps an existing [`http_body::Body`] implementation.
    ///
    /// Short-circuits `Body`-in-`Body` and `BoxBody`-in-`Body` nesting
    /// so repeated wrapping at layer boundaries does not stack virtual
    /// calls. Bodies that report `is_end_stream` up front collapse to
    /// [`Body::empty`].
    pub fn new<B>(mut body: B) -> Self
    where
        B: http_body::Body<Data = bytes::Bytes> + Send + Sync + 'static,
        B::Error: Into<crate::BoxError>,
    {
        if body.is_end_stream() {
            return Self::empty();
        }

        if let Some(body) = <dyn std::any::Any>::downcast_mut::<Body>(&mut body) {
            return std::mem::take(body);
        }

        if let Some(body) = <dyn std::any::Any>::downcast_mut::<BoxBody>(&mut body) {
            return Self::from_kind(Kind::Wrap(std::mem::take(body)));
        }

        let body = body.map_err(Into::into).boxed();

        Self::from_kind(Kind::Wrap(body))
    }
}

impl Default for Body {
    fn default() -> Self {
        Self::empty()
    }
}

impl http_body::Body for Body {
    type Data = bytes::Bytes;
    type Error = crate::BoxError;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match &mut self.kind {
            Kind::Empty => Poll::Ready(None),
            Kind::Wrap(body) => Pin::new(body).poll_frame(cx),
        }
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match &self.kind {
            Kind::Empty => http_body::SizeHint::with_exact(0),
            Kind::Wrap(body) => body.size_hint(),
        }
    }

    fn is_end_stream(&self) -> bool {
        match &self.kind {
            Kind::Empty => true,
            Kind::Wrap(body) => body.is_end_stream(),
        }
    }
}
