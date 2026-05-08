//! Server abstraction: what `ApiClient` prefixes requests with.
//!
//! A [`Server`] is anything that can produce a base URL for a request.
//! The trait exists purely as an entry bridge — [`crate::ApiClient`]
//! eagerly resolves the URL at construction time and stores it as an
//! [`Arc<str>`], so `Server` doesn't leak into `ApiClient`'s type
//! parameters. The blanket impls on `&str` / `String` / `Arc<str>`
//! mean the same constructor accepts a bare URL literal or a richer
//! generated `ServerOption*` value.

use std::{borrow::Cow, sync::Arc};

use crate::{MakeRequest, Operation, Request};

/// Produces a base URL for every outgoing request.
///
/// The returned URL is prepended to the relative URI that
/// [`MakeRequest::make_request`] emits. Implementations are free to
/// materialise the URL on every call (e.g. when template variables are
/// in play) or return a borrowed slice (when the URL is constant).
pub trait Server {
    /// Base URL, with or without a trailing slash. `ApiClient` strips
    /// at most one trailing slash before concatenation, so both forms
    /// round-trip.
    fn base_url(&self) -> Cow<'_, str>;
}

impl Server for str {
    fn base_url(&self) -> Cow<'_, str> {
        Cow::Borrowed(self)
    }
}

impl Server for String {
    fn base_url(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.as_str())
    }
}

impl Server for Arc<str> {
    fn base_url(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.as_ref())
    }
}

impl<T> Server for &T
where
    T: Server + ?Sized,
{
    fn base_url(&self) -> Cow<'_, str> {
        (**self).base_url()
    }
}

impl<T> Server for Box<T>
where
    T: Server + ?Sized,
{
    fn base_url(&self) -> Cow<'_, str> {
        (**self).base_url()
    }
}

/// Operation wrapper that routes this one call against a specific
/// base URL, overriding whatever the hosting [`crate::ApiClient`]
/// would otherwise prepend.
///
/// The wrapper stores the base URL as an [`Arc<str>`] resolved at
/// construction time, so [`Server`] stays off its type parameters.
/// `WithServer` implements [`MakeRequest`] + [`Operation`] by
/// delegating to the inner operation and rewriting the resulting
/// [`Request`]'s URI onto `base_url`. Downstream, `ApiClient` sees an
/// absolute URI and lets it pass through — that's how the override
/// bypasses the client's own server.
pub struct WithServer<Op> {
    op: Op,
    base_url: Arc<str>,
}

impl<Op> WithServer<Op> {
    /// Pairs an operation with an override server. Accepts anything
    /// that implements [`Server`] — a bare URL string, a generated
    /// `ServerOption*`, etc. — and materialises its base URL once.
    pub fn new<Srv: Server>(op: Op, server: Srv) -> Self {
        Self {
            op,
            base_url: Arc::from(server.base_url().as_ref()),
        }
    }

    /// Returns the wrapped operation and resolved base URL, consuming
    /// the wrapper.
    pub fn into_parts(self) -> (Op, Arc<str>) {
        (self.op, self.base_url)
    }
}

impl<Op> MakeRequest for WithServer<Op>
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
            let req = self.op.make_request().await?;
            Ok(rewrite_base_url(req, self.base_url.as_ref()))
        }
    }
}

impl<Op> Operation for WithServer<Op>
where
    Op: Operation + Send,
{
    type Response = Op::Response;
}

/// Rewrites `req`'s URI onto `base_url`, dropping whatever scheme /
/// authority the inner op might have baked in. Shared between
/// [`WithServer`] and [`crate::ApiClient`]'s default prefixing path.
pub(crate) fn rewrite_base_url(req: Request, base_url: &str) -> Request {
    let (mut parts, body) = req.into_parts();
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(ToString::to_string)
        .unwrap_or_default();
    let combined = format!("{}{}", trim_trailing_slash(base_url), path_and_query);
    if let Ok(new_uri) = combined.parse() {
        parts.uri = new_uri;
    }
    Request::from_parts(parts, body)
}

/// Prefixes `req`'s URI with `base_url` when the URI is still relative.
/// Absolute URIs (with a scheme) are returned as-is — this is how
/// [`WithServer`] bypasses the `ApiClient`'s own server.
pub(crate) fn prefix_base_url(req: Request, base_url: &str) -> Request {
    if req.uri().scheme().is_some() {
        return req;
    }
    rewrite_base_url(req, base_url)
}

/// Strips one trailing `/` so combining with a path that starts with
/// `/` doesn't produce a doubled separator.
fn trim_trailing_slash(s: &str) -> &str {
    s.strip_suffix('/').unwrap_or(s)
}
