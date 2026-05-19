//! Backend-compatibility adapters.
//!
//! Each submodule is feature-gated and turns a third-party HTTP client
//! into a [`tower::Service`] that speaks the runtime's [`crate::Request`]
//! / [`http::Response<_>`] pair. Keeping these adapters under one parent
//! module separates "transport interop" from the runtime's core
//! traits ([`crate::MakeRequest`], [`crate::ParseResponse`],
//! [`crate::ApiClient`]).

#[cfg(feature = "reqwest")]
pub mod reqwest;
