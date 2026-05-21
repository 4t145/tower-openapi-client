//! Generates the GitHub REST client from the upstream OpenAPI spec
//! vendored as a git submodule under `../github-openapi`. The submodule
//! tracks `https://github.com/github/rest-api-description`.

fn main() {
    toac_build::Builder::new("../github-openapi/descriptions/api.github.com/api.github.com.yaml")
        .output_file_name("github.rs")
        .use_chrono(true)
        .use_uuid(true)
        .use_base64_string(true)
        .emit();
}
