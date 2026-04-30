use std::fs;

#[test]
fn full_build_e2b() -> anyhow::Result<()> {
    let yaml = fs::read_to_string("tests/test_e2b_yaml_spec/openapi.yml")?;
    let spec = oas3::from_yaml(&yaml)?;
    let tokens = tower_openapi_client::build(&spec)?;
    syn::parse_file(&tokens.to_string())
        .map_err(|e| anyhow::anyhow!("parse fail: {e}\n---\n{tokens}"))?;
    Ok(())
}

#[test]
fn full_build_petstore() -> anyhow::Result<()> {
    let json = fs::read_to_string("tests/test_petstore31_json_spec/openapi.json")?;
    let spec = oas3::from_json(&json)?;
    let tokens = tower_openapi_client::build(&spec)?;
    syn::parse_file(&tokens.to_string())
        .map_err(|e| anyhow::anyhow!("parse fail: {e}\n---\n{tokens}"))?;
    Ok(())
}
