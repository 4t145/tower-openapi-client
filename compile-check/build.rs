//! Generates Rust client code from `fixtures/mini.yml` into
//! `$OUT_DIR/generated.rs`. The sibling `src/lib.rs` includes that file
//! so the actual Rust compiler type-checks the generator's output as
//! part of the normal workspace build.

use std::{env, fs, path::PathBuf};

const FIXTURE: &str = "fixtures/mini.yml";

fn main() {
    println!("cargo:rerun-if-changed={FIXTURE}");
    println!("cargo:rerun-if-changed=build.rs");

    let yaml = fs::read_to_string(FIXTURE).expect("read fixture");
    let spec = oas3::from_yaml(&yaml).expect("parse OpenAPI spec");
    let options = toac_build::BuildOptions {
        use_chrono: true,
        use_uuid: true,
        use_base64_string: true,
    };
    let tokens = toac_build::build_with(&spec, options).expect("generate client code");

    // Pretty-print through `prettyplease` so compile errors, if any,
    // come with readable line numbers.
    let file: syn::File = syn::parse_file(&tokens.to_string()).expect("generator emits valid Rust");
    let rendered = prettyplease::unparse(&file);

    let out_path: PathBuf =
        PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("generated.rs");
    fs::write(&out_path, rendered).expect("write generated module");
}
