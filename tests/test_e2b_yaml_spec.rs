use std::fs;

#[test]
fn test_e2b_yaml_spec() -> anyhow::Result<()> {
    let yaml = fs::read_to_string("tests/test_e2b_yaml_spec/openapi.yml")?;
    let spec = oas3::from_yaml(&yaml)?;
    let _regenerated_yaml = oas3::to_yaml(&spec)?;
    Ok(())
}

#[test]
fn components_generate_parsable_rust() -> anyhow::Result<()> {
    let yaml = fs::read_to_string("tests/test_e2b_yaml_spec/openapi.yml")?;
    let spec = oas3::from_yaml(&yaml)?;
    let tokens = tower_openapi_client::build_components(&spec)?;
    let rendered = tokens.to_string();
    let parsed = syn::parse_file(&rendered).map_err(|err| {
        anyhow::anyhow!("generated tokens failed to parse as Rust: {err}\n---\n{rendered}")
    })?;
    if std::env::var_os("DUMP_GEN").is_some() {
        eprintln!("{}", prettyplease::unparse(&parsed));
    }
    Ok(())
}
