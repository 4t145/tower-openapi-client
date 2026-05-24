use indoc::indoc;

/// Parses an inline OpenAPI spec and returns the generated component module
/// rendered as Rust source.
fn generate(spec_yaml: &str) -> String {
    let spec = oas3::from_yaml(spec_yaml).expect("spec parses");
    let tokens = toac_build::build_components(&spec).expect("codegen");
    let file = syn::parse_file(&tokens.to_string()).expect("valid Rust");
    prettyplease::unparse(&file)
}

#[test]
fn one_of_without_discriminator_is_untagged() {
    let rendered = generate(indoc! {r##"
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
    "##});

    assert!(
        rendered.contains("#[serde(untagged)]"),
        "missing untagged: {rendered}"
    );
    assert!(rendered.contains("pub enum Pet"));
    assert!(rendered.contains("Cat(Cat)"));
    assert!(rendered.contains("Dog(Dog)"));
    assert!(rendered.contains("impl ::std::convert::From<Cat> for Pet"));
    assert!(rendered.contains("impl ::std::convert::TryFrom<Pet> for Cat"));
}

#[test]
fn one_of_with_discriminator_is_internally_tagged() {
    let rendered = generate(indoc! {r##"
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
    "##});

    assert!(
        rendered.contains(r#"#[serde(tag = "kind")]"#),
        "missing internal tag: {rendered}"
    );
    assert!(
        rendered.contains(r#"#[serde(rename = "cat_wire")]"#),
        "missing discriminator rename: {rendered}"
    );
    assert!(!rendered.contains("#[serde(untagged)]"));
}

#[test]
fn any_of_is_untagged() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info:
          title: t
          version: "0"
        components:
          schemas:
            A:
              type: object
              properties:
                a: { type: string }
            B:
              type: object
              properties:
                b: { type: string }
            Either:
              anyOf:
                - $ref: "#/components/schemas/A"
                - $ref: "#/components/schemas/B"
    "##});

    assert!(rendered.contains("#[serde(untagged)]"));
    assert!(rendered.contains("pub enum Either"));
    assert!(rendered.contains("A(A)"));
    assert!(rendered.contains("B(B)"));
}

#[test]
fn inline_one_of_member_is_hoisted() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info:
          title: t
          version: "0"
        components:
          schemas:
            Event:
              oneOf:
                - type: object
                  properties:
                    kind: { type: string }
                - type: string
    "##});

    assert!(rendered.contains("pub enum Event"));
    // inline object hoisted to its own type
    assert!(
        rendered.contains("EventVariant0") || rendered.contains("pub struct EventVariant"),
        "expected a hoisted inline object variant: {rendered}"
    );
}

#[test]
fn primitive_sum_gets_display_impl() {
    // `oneOf: [integer, string]` is the canonical "id can be numeric or
    // string" pattern. Path-parameter rendering calls `ToString` on the
    // value, so the generated enum must implement `Display`.
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info:
          title: t
          version: "0"
        components:
          schemas:
            WorkflowId:
              oneOf:
                - type: integer
                - type: string
    "##});

    assert!(
        rendered.contains("impl ::std::fmt::Display for WorkflowId"),
        "expected Display impl for primitive sum: {rendered}"
    );
}

#[test]
fn complex_sum_has_no_display_impl() {
    // We can't statically prove that struct variants implement `Display`,
    // so the impl must be omitted to avoid generating uncompilable code.
    let rendered = generate(indoc! {r##"
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
            Pet:
              oneOf:
                - $ref: "#/components/schemas/Cat"
                - type: string
    "##});

    assert!(
        !rendered.contains("impl ::std::fmt::Display for Pet"),
        "Display must not be emitted when a variant wraps a struct: {rendered}"
    );
}

#[test]
fn duplicate_variant_inner_type_skips_redundant_impls() {
    // Both variants wrap the same inner type (String). We keep the enum
    // compiling — only the first From/TryFrom pair is emitted.
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info:
          title: t
          version: "0"
        components:
          schemas:
            StringOrString:
              oneOf:
                - type: string
                - type: string
    "##});

    let from_impls = rendered
        .matches("impl ::std::convert::From<String>")
        .count();
    assert_eq!(
        from_impls, 1,
        "expected exactly one From<String> impl: {rendered}"
    );
}
