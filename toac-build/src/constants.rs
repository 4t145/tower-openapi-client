//! Central place for names and paths that tie the generator to the
//! runtime crate.
//!
//! Having these in one module means "what's the runtime crate called?"
//! is answered by changing a single constant — useful both for renames
//! and for future work that lets callers override the target crate
//! (e.g. re-exporting `toac` under a different name inside a larger
//! application crate).

use proc_macro2::Span;
use syn::parse_quote;

/// Name of the runtime crate the generated code references. Emitted
/// paths look like `::<RUNTIME_CRATE>::ItemName` so consumers can
/// resolve them against their `[dependencies]`.
pub const RUNTIME_CRATE: &str = "toac";

/// Builds a `::<RUNTIME_CRATE>::item` path.
pub fn runtime_path(item: &str) -> syn::Path {
    let crate_ident = syn::Ident::new(RUNTIME_CRATE, Span::call_site());
    let item_ident = syn::Ident::new(item, Span::call_site());
    parse_quote!(::#crate_ident::#item_ident)
}

/// Returns the runtime crate name as a `syn::Ident`, useful when the
/// caller wants to compose a longer path manually.
pub fn runtime_crate_ident() -> syn::Ident {
    syn::Ident::new(RUNTIME_CRATE, Span::call_site())
}

/// Path to [`toac::body::Body`], the fixed body type used on every
/// generated request.
pub fn runtime_body_path() -> syn::Path {
    let crate_ident = syn::Ident::new(RUNTIME_CRATE, Span::call_site());
    parse_quote!(::#crate_ident::body::Body)
}
