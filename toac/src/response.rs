/// HTTP response type consumed by [`crate::ParseResponse`] implementations.
///
/// Fixing the body to [`crate::body::Body`] lets every generated
/// `{Op}Response` decode the same concrete type while remaining agnostic
/// to the underlying transport.
pub type Response = ::http::Response<crate::body::Body>;
