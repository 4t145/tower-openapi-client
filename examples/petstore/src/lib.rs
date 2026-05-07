//! Library surface for the example binary. It consists entirely of the
//! generator's output included through `toac::include_client!`, which
//! in turn is produced by `build.rs` from the real Petstore spec.
//!
//! The `#![allow(...)]` list silences stylistic lints against generated
//! code that would otherwise mask the signal from the example itself.

#![allow(
    clippy::manual_async_fn,
    clippy::needless_return,
    clippy::single_match,
    clippy::match_single_binding,
    clippy::too_many_arguments,
    clippy::large_enum_variant,
    dead_code
)]

toac::include_client!("petstore");
