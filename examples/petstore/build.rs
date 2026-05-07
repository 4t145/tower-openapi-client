//! Generates the Petstore client from the real-world OpenAPI spec in
//! the repository's test fixtures. All of the ceremony (reading,
//! parsing, pretty-printing, writing) lives in [`toac_build::Builder`].

fn main() {
    toac_build::Builder::new("../../toac-build/tests/test_petstore31_json_spec/openapi.json")
        .output_file_name("petstore.rs")
        .use_chrono(true)
        .use_uuid(true)
        .use_base64_string(true)
        .emit();
}
