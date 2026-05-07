//! Runs the code generator against `fixtures/mini.yml` so the sibling
//! `src/lib.rs` can include its output. All of the boring plumbing
//! (reading, parsing, pretty-printing, writing) lives inside
//! [`toac_build::Builder`].

fn main() {
    toac_build::Builder::new("fixtures/mini.yml")
        .use_chrono(true)
        .use_uuid(true)
        .use_base64_string(true)
        .emit();
}
