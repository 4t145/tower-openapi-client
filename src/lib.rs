//! Tower-compatible client code generator for OpenAPI 3 specifications.
//!
//! Consumes [`oas3::Spec`] values and produces a [`proc_macro2::TokenStream`]
//! containing the generated Rust client. The current surface covers the
//! `components/schemas` section; other parts of the spec are staged in
//! follow-up work.

pub mod components;
pub mod docs;
pub mod generator;
pub mod naming;
pub mod operations;
#[cfg(feature = "runtime")]
pub mod runtime;

use oas3::spec;

pub use generator::Generator;

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
    let mut generator = Generator::new(spec);
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
