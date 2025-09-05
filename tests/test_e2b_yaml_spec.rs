use std::fs;

#[test]
fn test_e2b_yaml_spec()  -> anyhow::Result<()> {
    let yaml = fs::read_to_string("tests/test_e2b_yaml_spec/openapi.yml")?;
    let spec = oas3::from_yaml(&yaml)?;
    let _regenerated_yaml = oas3::to_yaml(&spec)?;
    Ok(())
}