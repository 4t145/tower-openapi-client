//! Generates the Daytona client from the upstream OpenAPI spec stored
//! under `fixtures/openapi.json`. The fixture is a verbatim copy of
//! `https://www.daytona.io/docs/openapi.json`; re-download and check
//! the diff if the live spec moves on.

fn main() {
    toac_build::Builder::new("fixtures/openapi.json")
        .output_file_name("daytona.rs")
        .use_chrono(true)
        .use_uuid(true)
        .use_base64_string(true)
        .emit();
}
