//! Codegen shape tests for the spec-level metadata renderer that
//! projects `info`, `externalDocs`, etc. into outer `#[doc]` attributes
//! attached to the generated `pub mod spec` marker module.

use indoc::indoc;

fn generate(spec_yaml: &str) -> String {
    let spec = oas3::from_yaml(spec_yaml).expect("spec parses");
    let tokens = toac_build::build(&spec).expect("codegen");
    let file = syn::parse_file(&tokens.to_string()).expect("valid Rust");
    prettyplease::unparse(&file)
}

/// Pulls the outer doc lines that precede `pub mod spec` — i.e. the
/// spec-metadata block emitted by the docs renderer.
fn inner_doc_lines(rendered: &str) -> Vec<String> {
    let mut block: Vec<String> = Vec::new();
    for line in rendered.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("///") else {
            if !block.is_empty() {
                if trimmed.starts_with("pub mod spec") {
                    return block;
                }
                block.clear();
            }
            continue;
        };
        block.push(rest.trim().to_owned());
    }
    block
}

#[test]
fn info_block_renders_title_summary_description_version() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info:
          title: Swagger Petstore
          summary: A short summary
          description: |-
            Long-form description.
            Spans multiple lines.
          version: "1.2.3"
        paths: {}
    "##});

    let docs = inner_doc_lines(&rendered);
    assert!(docs.iter().any(|l| l == "# Swagger Petstore"), "{docs:?}");
    assert!(docs.iter().any(|l| l == "A short summary"), "{docs:?}");
    assert!(
        docs.iter().any(|l| l == "Long-form description."),
        "{docs:?}"
    );
    assert!(
        docs.iter().any(|l| l == "Spans multiple lines."),
        "{docs:?}"
    );
    assert!(
        docs.iter().any(|l| l == "- **Version:** `1.2.3`"),
        "{docs:?}"
    );
}

#[test]
fn contact_license_terms_and_external_docs_render() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info:
          title: API
          version: "0"
          termsOfService: https://example.com/tos
          contact:
            name: API Team
            url: https://example.com/contact
            email: api@example.com
          license:
            name: Apache 2.0
            identifier: Apache-2.0
            url: https://www.apache.org/licenses/LICENSE-2.0.html
        externalDocs:
          description: Find out more
          url: https://example.com/docs
        paths: {}
    "##});

    let docs = inner_doc_lines(&rendered);
    assert!(
        docs.iter()
            .any(|l| l == "- **Terms of Service:** <https://example.com/tos>"),
        "{docs:?}"
    );
    assert!(
        docs.iter().any(|l| l.starts_with("- **Contact:**")
            && l.contains("API Team")
            && l.contains("<https://example.com/contact>")
            && l.contains("<api@example.com>")),
        "{docs:?}"
    );
    assert!(
        docs.iter().any(|l| l.starts_with("- **License:**")
            && l.contains("Apache 2.0")
            && l.contains("`Apache-2.0`")
            && l.contains("<https://www.apache.org/licenses/LICENSE-2.0.html>")),
        "{docs:?}"
    );
    assert!(
        docs.iter().any(|l| l == "# External Documentation"),
        "{docs:?}"
    );
    assert!(docs.iter().any(|l| l == "Find out more"), "{docs:?}");
    assert!(
        docs.iter().any(|l| l == "See <https://example.com/docs>."),
        "{docs:?}"
    );
}

#[test]
fn minimal_spec_emits_only_title_and_version() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: Minimal, version: "0" }
        paths: {}
    "##});

    let docs = inner_doc_lines(&rendered);
    assert_eq!(docs[0], "# Minimal");
    assert!(docs.contains(&"- **Version:** `0`".to_owned()));
    assert!(!docs.iter().any(|l| l.contains("Contact")));
    assert!(!docs.iter().any(|l| l.contains("License")));
    assert!(!docs.iter().any(|l| l.contains("External Documentation")));
}
