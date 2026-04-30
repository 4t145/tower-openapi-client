use std::fs;

#[test]
fn test_petstore31_json_spec() -> anyhow::Result<()> {
    let json = fs::read_to_string("tests/test_petstore31_json_spec/openapi.json")?;
    let spec = oas3::from_json(&json)?;
    let _regenerated_yaml = oas3::to_json(&spec)?;
    let tokens = tower_openapi_client::build(&spec)?;
    let rendered = tokens.to_string();
    let parsed = syn::parse_file(&rendered).map_err(|err| {
        anyhow::anyhow!("generated tokens failed to parse as Rust: {err}\n---\n{rendered}")
    })?;
    let pretty = prettyplease::unparse(&parsed);
    let out_path = std::env::temp_dir().join("test_petstore31_json_spec.rs");
    fs::write(&out_path, &pretty)?;
    eprintln!("parsed output written to {}", out_path.display());

    Ok(())
}
