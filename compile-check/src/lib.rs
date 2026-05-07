//! Includes the generator's output so rustc type-checks it.
//!
//! The generated code is written by `build.rs` into `$OUT_DIR/mini.rs`
//! and consists of two inner modules: `components` and `operations`.
//! Any compile error in the generator surfaces as a build failure of
//! this crate — which is exactly what we want out of a compile check.
//!
//! Lints silenced below are stylistic observations against generated
//! code; they don't affect correctness and the generator emits the
//! long-form constructs deliberately (e.g. `impl Future + Send` is
//! required to match the runtime trait signature, which `async fn`
//! cannot express).

#![allow(
    clippy::manual_async_fn,
    clippy::needless_return,
    clippy::single_match,
    clippy::match_single_binding
)]

toac::include_client!("mini");
