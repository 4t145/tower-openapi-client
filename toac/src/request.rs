pub mod parameter;

/// HTTP request type produced by [`crate::MakeRequest`] implementations.
///
/// Fixing the body to [`crate::body::Body`] keeps generated code
/// monomorphic and gives every transport layer a single type to accept.
pub type Request = ::http::Request<crate::body::Body>;
