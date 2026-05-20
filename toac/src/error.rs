/// Erased error used across the runtime surface.
///
/// The `Send + Sync` bounds match the convention of `tower::BoxError`
/// and `hyper::Error`, so transport layers can forward their native
/// errors into [`body::Body`] and [`DecodeError::BodyRead`] without
/// extra boxing.
///
/// [`body::Body`]: crate::body::Body
/// [`DecodeError::BodyRead`]: crate::DecodeError::BodyRead
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Concrete `std::error::Error` wrapper around a [`BoxError`].
///
/// Generated [`MakeRequest`][crate::MakeRequest] impls need an `Error`
/// type when they combine multiple failure surfaces (e.g. parameter
/// encoding and body codec), and `BoxError` itself is not an `Error`
/// because `dyn Error` is unsized. This newtype gives codegen a single
/// `Error + Send + Sync + 'static` value that any `Into<BoxError>` source
/// can be folded into.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct EncodeRequestError(#[from] BoxError);

impl EncodeRequestError {
    /// Wraps any `Into<BoxError>` source. Equivalent to `From::from`,
    /// kept as an explicit name for codegen readability.
    pub fn new<E: Into<BoxError>>(error: E) -> Self {
        Self(error.into())
    }

    /// Returns the inner [`BoxError`]. Useful when the caller wants to
    /// downcast to a concrete codec error for diagnostics.
    pub fn into_inner(self) -> BoxError {
        self.0
    }
}
