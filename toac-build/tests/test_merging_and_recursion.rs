use indoc::indoc;

fn generate(spec_yaml: &str) -> String {
    let spec = oas3::from_yaml(spec_yaml).expect("spec parses");
    let tokens = toac_build::build_components(&spec).expect("codegen");
    let file = syn::parse_file(&tokens.to_string()).expect("valid Rust");
    prettyplease::unparse(&file)
}

#[test]
fn all_of_two_refs_merges_their_properties() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            Animal:
              type: object
              required: [species]
              properties:
                species: { type: string }
            Pedigreed:
              type: object
              required: [breed]
              properties:
                breed: { type: string }
            Dog:
              allOf:
                - $ref: "#/components/schemas/Animal"
                - $ref: "#/components/schemas/Pedigreed"
                - type: object
                  required: [name]
                  properties:
                    name: { type: string }
    "##});

    assert!(
        rendered.contains("pub struct Dog"),
        "Dog missing: {rendered}"
    );
    // All three source fields land on Dog as non-optional (all required).
    assert!(rendered.contains("pub species: String"));
    assert!(rendered.contains("pub breed: String"));
    assert!(rendered.contains("pub name: String"));
}

#[test]
fn all_of_with_optional_and_required_unions_required() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            Base:
              type: object
              properties:
                id: { type: string }
            Extended:
              allOf:
                - $ref: "#/components/schemas/Base"
                - type: object
                  required: [id, label]
                  properties:
                    label: { type: string }
    "##});

    assert!(rendered.contains("pub struct Extended"));
    // `id` is required via the Extended side -> non-Option
    assert!(
        rendered.contains("pub id: String"),
        "expected required id: {rendered}"
    );
    assert!(rendered.contains("pub label: String"));
}

#[test]
fn self_recursive_field_is_boxed() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            Node:
              type: object
              required: [value]
              properties:
                value: { type: string }
                next:
                  $ref: "#/components/schemas/Node"
    "##});

    assert!(rendered.contains("pub struct Node"));
    assert!(
        rendered.contains("Box<Node>"),
        "self-reference should be boxed: {rendered}"
    );
}

#[test]
fn self_recursive_through_vec_is_not_boxed() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            Tree:
              type: object
              required: [value]
              properties:
                value: { type: string }
                children:
                  type: array
                  items:
                    $ref: "#/components/schemas/Tree"
    "##});

    assert!(rendered.contains("pub struct Tree"));
    assert!(rendered.contains("Vec<Tree>"));
    // Vec already indirects the size; no extra Box needed.
    assert!(
        !rendered.contains("Vec<Box<Tree>>") && !rendered.contains("Box<Tree>"),
        "Vec<Tree> does not need boxing: {rendered}"
    );
}

#[test]
fn mutual_recursion_gets_boxed() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            A:
              type: object
              required: [next]
              properties:
                next:
                  $ref: "#/components/schemas/B"
            B:
              type: object
              required: [back]
              properties:
                back:
                  $ref: "#/components/schemas/A"
    "##});

    assert!(
        rendered.contains("Box<B>") || rendered.contains("Box<A>"),
        "at least one mutual edge must be boxed: {rendered}"
    );
}
