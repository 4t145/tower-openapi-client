//! Generates the Petstore client from the real-world OpenAPI spec in
//! the repository's test fixtures. The output lands in
//! `$OUT_DIR/generated.rs` and is `include!`d by `src/lib.rs`, so rustc
//! type-checks the whole thing as part of building the example binary.

use std::{env, fs, path::PathBuf};

const SPEC_PATH: &str = "../../toac-build/tests/test_petstore31_json_spec/openapi.json";

fn main() {
    println!("cargo:rerun-if-changed={SPEC_PATH}");
    println!("cargo:rerun-if-changed=build.rs");

    let json = fs::read_to_string(SPEC_PATH).expect("read Petstore spec");
    let spec = oas3::from_json(&json).expect("parse OpenAPI spec");
    let options = toac_build::BuildOptions {
        use_chrono: true,
        use_uuid: true,
        use_base64_string: true,
    };
    let tokens = toac_build::build_with(&spec, options).expect("generate Petstore client");

    // Pretty-print so compile errors (if any) surface with human-readable
    // line numbers.
    let file: syn::File = syn::parse_file(&tokens.to_string()).expect("generator emits valid Rust");
    let rendered = prettyplease::unparse(&file);

    let out_path: PathBuf =
        PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("generated.rs");
    fs::write(&out_path, rendered).expect("write generated module");
}
