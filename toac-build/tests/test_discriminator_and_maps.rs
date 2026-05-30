//! Regression coverage for the discriminator and open-map codegen fixes.
//!
//! - A `oneOf` + `discriminator` whose variants declare the
//!   discriminator property themselves (the OpenAI "type"/"role"
//!   const-field pattern) must NOT lower to `#[serde(tag = "...")]`:
//!   serde would then write the tag property twice (once itself, once
//!   from the variant's own field), producing a duplicate key that the
//!   upstream API rejects. Such enums downgrade to `#[serde(untagged)]`
//!   and keep their fields, so serde discriminates on the fixed values.
//!
//! - A clean discriminator, where no variant declares the property,
//!   keeps the internally-tagged form (serde owns the tag).
//!
//! - Open maps (`additionalProperties`) lower to `BTreeMap`, not
//!   `HashMap`, so serialization order is deterministic.

use indoc::indoc;

fn generate(spec_yaml: &str) -> String {
    let spec = oas3::from_yaml(spec_yaml).expect("spec parses");
    let tokens = toac_build::build_components(&spec).expect("codegen");
    let file = syn::parse_file(&tokens.to_string()).expect("valid Rust");
    prettyplease::unparse(&file)
}

/// Variants carry the discriminator property (`role`) as their own field
/// — the shape that breaks internal tagging.
const SELF_FIELD_SPEC: &str = indoc! {r##"
    openapi: 3.1.0
    info:
      title: t
      version: "0"
    components:
      schemas:
        Msg:
          oneOf:
            - $ref: "#/components/schemas/UserMsg"
            - $ref: "#/components/schemas/AssistantMsg"
          discriminator:
            propertyName: role
        UserMsg:
          type: object
          required: [role, content]
          properties:
            role: { type: string, enum: [user] }
            content: { type: string }
        AssistantMsg:
          type: object
          required: [role, content]
          properties:
            role: { type: string, enum: [assistant] }
            content: { type: string }
"##};

/// Clean discriminator: variants do NOT declare the `kind` property, so
/// serde can own the tag.
const CLEAN_DISCRIMINATOR_SPEC: &str = indoc! {r##"
    openapi: 3.1.0
    info:
      title: t
      version: "0"
    components:
      schemas:
        Cat:
          type: object
          properties:
            meow: { type: string }
        Dog:
          type: object
          properties:
            bark: { type: string }
        Pet:
          oneOf:
            - $ref: "#/components/schemas/Cat"
            - $ref: "#/components/schemas/Dog"
          discriminator:
            propertyName: kind
            mapping:
              cat_wire: "#/components/schemas/Cat"
"##};

#[test]
fn discriminator_with_self_field_downgrades_to_untagged() {
    let rendered = generate(SELF_FIELD_SPEC);

    assert!(
        rendered.contains("#[serde(untagged)]"),
        "self-field discriminator should be untagged: {rendered}"
    );
    assert!(
        !rendered.contains(r#"#[serde(tag = "role")]"#),
        "must not keep internal tag on `role`: {rendered}"
    );

    // The variant structs keep their `role` field — serde no longer owns
    // it, so it must travel on the struct.
    let user = struct_body(&rendered, "UserMsg");
    assert!(
        user.contains("role"),
        "UserMsg must keep its `role` field: {user}"
    );
}

#[test]
fn clean_discriminator_stays_internally_tagged() {
    let rendered = generate(CLEAN_DISCRIMINATOR_SPEC);

    assert!(
        rendered.contains(r#"#[serde(tag = "kind")]"#),
        "clean discriminator should stay internally tagged: {rendered}"
    );
    assert!(
        rendered.contains(r#"#[serde(rename = "cat_wire")]"#),
        "discriminator mapping rename should survive: {rendered}"
    );
    assert!(!rendered.contains("#[serde(untagged)]"));
}

#[test]
fn open_maps_lower_to_btreemap() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info:
          title: t
          version: "0"
        components:
          schemas:
            Meta:
              type: object
              additionalProperties:
                type: string
    "##});

    assert!(
        rendered.contains("BTreeMap<String, String>"),
        "open map should be BTreeMap: {rendered}"
    );
    assert!(
        !rendered.contains("HashMap"),
        "open map must not use HashMap: {rendered}"
    );
}

/// Returns the source of the named struct's body, for substring checks.
fn struct_body<'a>(rendered: &'a str, name: &str) -> &'a str {
    let needle = format!("pub struct {name} {{");
    let start = rendered
        .find(&needle)
        .unwrap_or_else(|| panic!("struct {name} not found in:\n{rendered}"));
    let rest = &rendered[start..];
    let end = rest
        .find('}')
        .map(|i| start + i + 1)
        .unwrap_or(rendered.len());
    &rendered[start..end]
}
