//! Shared attribute prologues attached to every top-level generated module.
//!
//! The generator emits four sibling modules (`components`, `operations`,
//! `servers`, `security`) directly into the consumer's crate via
//! [`include!`]. Lint warnings inside that file are noisy and mostly
//! intrinsic to mechanical codegen — for example, every `From<Foo>` for
//! a deprecated `Foo` triggers `deprecated`, and the dedup'd
//! `TryFrom<Enum>` impls for sum types end with an `unreachable_patterns`
//! catch-all.
//!
//! The fix is to attach `#![allow(...)]` *inner* attributes to each
//! generated module so the silencing is scoped to the generator's own
//! output. Calls from user code into a deprecated item still warn —
//! inner attributes don't propagate across module boundaries.

use quote::quote;

/// Inner attributes injected at the top of every top-level generated
/// module.
///
/// The lints listed here are the ones that the codegen itself
/// produces — silencing them at user call sites would defeat their
/// purpose.
///
/// - `deprecated`: the spec marks types as deprecated, and the
///   generator still emits supporting impls (`From`, `TryFrom`,
///   re-exports) for them. The user-side warning fires on actual *use*
///   of the deprecated item; intra-module references shouldn't.
/// - `unreachable_patterns`: dedup of duplicate sum-enum inner types
///   leaves a fall-through arm in `TryFrom` impls that the compiler
///   correctly flags as unreachable when only one variant remains.
/// - `non_camel_case_types` / `non_snake_case`: real-world specs use
///   identifiers that don't always round-trip to Rust casing
///   conventions. We do best-effort normalisation in
///   [`crate::naming`], but leftovers shouldn't surface as user
///   warnings.
/// - `clippy::all` / `clippy::pedantic`: the generated file is not
///   meant to be hand-reviewed; clippy churn there only obscures the
///   user's own findings.
pub fn module_inner_attrs() -> proc_macro2::TokenStream {
    quote! {
        #![allow(deprecated)]
        #![allow(unreachable_patterns)]
        #![allow(non_camel_case_types)]
        #![allow(non_snake_case)]
        #![allow(clippy::all, clippy::pedantic)]
    }
}
