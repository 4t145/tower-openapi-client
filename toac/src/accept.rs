//! Request-level `Accept` override.
//!
//! Generated [`MakeRequest`] impls auto-emit an `Accept` header that
//! enumerates every response media type declared in the spec, in
//! codec-priority order. That covers the common case but leaves no room
//! for callers who want to *steer* content negotiation toward one
//! specific branch — e.g. asking OpenAI's chat completion endpoint for
//! `text/event-stream` to land on the streaming response.
//!
//! [`WithAccept`] is the runtime adapter that drives this: wrap any
//! operation value, give it the [`http::HeaderValue`] you want on the
//! wire, and the wrapper rewrites the `Accept` header after the inner
//! op finishes its `MakeRequest::make_request` work. Because the
//! override happens after `make_request` returns, the inner op's other
//! header / body / extension wiring is preserved as-is.
//!
//! Usage from generated code:
//!
//! ```ignore
//! use ::http::HeaderValue;
//! use toac::WithAccept;
//!
//! let op = create_chat_completion::Request { body: ... };
//! let op = WithAccept::new(op, HeaderValue::from_static("text/event-stream"));
//! client.call(op).await?;
//! ```

use ::http::{HeaderValue, header::ACCEPT};

use crate::{MakeRequest, Operation, Request};

/// Operation wrapper that overrides the `Accept` header the inner op
/// would emit.
///
/// The wrapper passes the operation through unchanged otherwise — same
/// method, URI, body, extensions, and other headers. Only `Accept` is
/// replaced (any pre-existing `Accept` header is removed first, so the
/// override is single-valued on the wire).
#[derive(Debug, Clone)]
pub struct WithAccept<Op> {
    op: Op,
    accept: HeaderValue,
}

impl<Op> WithAccept<Op> {
    /// Pairs an operation with the `Accept` header value to send.
    pub fn new(op: Op, accept: HeaderValue) -> Self {
        Self { op, accept }
    }

    /// Returns the wrapped operation and the override value, consuming
    /// the wrapper.
    pub fn into_parts(self) -> (Op, HeaderValue) {
        (self.op, self.accept)
    }
}

impl<Op> MakeRequest for WithAccept<Op>
where
    Op: MakeRequest + Send,
{
    type Error = Op::Error;

    // The trait's `+ Send` bound needs the explicit `impl Future + Send`
    // form; `async fn` would not carry it through.
    #[allow(clippy::manual_async_fn)]
    fn make_request(
        self,
    ) -> impl std::future::Future<Output = Result<Request, Self::Error>> + Send {
        async move {
            let mut req = self.op.make_request().await?;
            let headers = req.headers_mut();
            headers.remove(ACCEPT);
            headers.insert(ACCEPT, self.accept);
            Ok(req)
        }
    }
}

impl<Op> Operation for WithAccept<Op>
where
    Op: Operation + Send,
{
    type Response = Op::Response;
}
