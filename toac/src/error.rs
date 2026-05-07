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
