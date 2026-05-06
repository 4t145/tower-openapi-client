//! Code generator for the `toac` Tower-compatible OpenAPI client runtime.
//!
//! Consumes [`oas3::Spec`] values and produces a [`proc_macro2::TokenStream`]
//! containing the generated Rust client. The output links against the
//! `toac` crate at runtime; this crate is typically invoked from a
//! consumer's `build.rs`.

pub mod components;
pub mod constants;
pub mod docs;
pub mod generator;
pub mod naming;
pub mod operations;

use oas3::spec;

pub use generator::{BuildOptions, Generator};

/// Errors produced while generating client code from a spec.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An [`oas3`] error surfaced while parsing or walking the spec.
    #[error("OpenAPI specification error: {0}")]
    Spec(#[from] spec::Error),

    /// Failed to resolve a `$ref` path against the spec.
    #[error("reference error: {0}")]
    Ref(#[from] spec::RefError),

    /// A spec feature is recognised but not yet implemented by the generator.
    #[error("unsupported feature: {0}")]
    Unsupported(String),
}

/// Generates the full Rust client code — `components`, `operations`,
/// and every later stage — for the given spec.
///
/// # Errors
///
/// Returns [`Error::Ref`] when a `$ref` cannot be resolved and
/// [`Error::Unsupported`] when the generator meets a construct it does
/// not handle yet.
pub fn build(spec: &oas3::Spec) -> Result<proc_macro2::TokenStream, Error> {
    build_with(spec, BuildOptions::default())
}

/// Generates the full Rust client code with configurable codegen
/// options.
///
/// Use this entry point when you want the generator to emit
/// `chrono`/`uuid`/`Base64String` types for the corresponding
/// `format` annotations. The flags on [`BuildOptions`] are opt-in so
/// that the default surface stays compatible with spec consumers who
/// prefer plain `String` values.
///
/// # Errors
///
/// Same as [`build`].
pub fn build_with(
    spec: &oas3::Spec,
    options: BuildOptions,
) -> Result<proc_macro2::TokenStream, Error> {
    let mut generator = Generator::with_options(spec, options);
    generator.emit_components()?;
    generator.emit_operations()?;

    let components = generator.finish_components();
    let operations = generator.finish_operations();
    Ok(quote::quote! {
        #components
        #operations
    })
}

/// Convenience entry point that generates only the `components` module.
///
/// # Errors
///
/// Same as [`build`].
pub fn build_components(spec: &oas3::Spec) -> Result<proc_macro2::TokenStream, Error> {
    let mut generator = Generator::new(spec);
    generator.emit_components()?;
    Ok(generator.finish_components())
}
