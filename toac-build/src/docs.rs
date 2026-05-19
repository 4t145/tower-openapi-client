//! Helpers for turning OpenAPI descriptive metadata into Rust doc attributes.

use quote::quote;
use syn::parse_quote;

/// Maximum number of examples to embed in a single type's doc comment.
///
/// Large spec files can attach many examples; embedding them all would bloat
/// generated output. A small cap keeps the rustdoc legible while still
/// showing the common shapes.
const MAX_DOC_EXAMPLES: usize = 3;

/// Returns a `#[doc = "<line>"]` attribute for the given text.
fn doc_attr(line: &str) -> syn::Attribute {
    parse_quote!(#[doc = #line])
}

/// Appends `#[doc = "..."]` attributes describing an OpenAPI schema's title,
/// description, and examples.
pub fn push_schema_docs(
    attrs: &mut Vec<syn::Attribute>,
    title: Option<&str>,
    description: Option<&str>,
    examples: &[serde_json::Value],
) {
    if let Some(title) = title {
        let line = format!(" # {title}");
        attrs.push(doc_attr(&line));
    }

    if let Some(description) = description {
        push_multiline(attrs, description);
    }

    push_examples(attrs, examples);
}

/// Appends a single-line description as a `#[doc = "..."]` attribute.
pub fn push_field_docs(attrs: &mut Vec<syn::Attribute>, description: Option<&str>) {
    if let Some(description) = description {
        push_multiline(attrs, description);
    }
}

fn push_multiline(attrs: &mut Vec<syn::Attribute>, text: &str) {
    for line in text.lines() {
        let formatted = format!(" {line}");
        attrs.push(doc_attr(&formatted));
    }
}

fn push_examples(attrs: &mut Vec<syn::Attribute>, examples: &[serde_json::Value]) {
    if examples.is_empty() {
        return;
    }
    attrs.push(doc_attr(""));
    attrs.push(doc_attr(" # Examples"));

    for example in examples.iter().take(MAX_DOC_EXAMPLES) {
        let rendered = serde_json::to_string_pretty(example).unwrap_or_default();
        attrs.push(doc_attr(""));
        attrs.push(doc_attr(" ```json"));
        for line in rendered.lines() {
            let formatted = format!(" {line}");
            attrs.push(doc_attr(&formatted));
        }
        attrs.push(doc_attr(" ```"));
    }
}

/// Returns a `#[deprecated]` attribute when `deprecated` is `Some(true)`.
pub fn deprecated_attr(deprecated: Option<bool>) -> Option<syn::Attribute> {
    matches!(deprecated, Some(true)).then(|| parse_quote!(#[deprecated]))
}

/// Renders the spec's descriptive metadata as a documentation-only
/// `pub mod spec` block whose outer doc attributes describe the API.
///
/// Pulls from the spec's `info` block (`title`, `summary`, `description`,
/// `version`, `termsOfService`, `contact`, `license`) and the top-level
/// `externalDocs`. Empty fields are skipped so the output stays readable.
///
/// Outer attributes (rather than inner `#![doc]`) keep the metadata
/// includable from anywhere — `toac::include_client!` dumps the
/// generated tokens at the call site, where inner attributes would
/// require us to control the surrounding module's prelude.
pub fn spec_metadata_docs(spec: &oas3::Spec) -> proc_macro2::TokenStream {
    let mut lines: Vec<String> = Vec::new();
    let info = &spec.info;

    push_doc_paragraph(&mut lines, Some(&format!("# {}", info.title)));
    push_doc_paragraph(&mut lines, info.summary.as_deref());
    push_doc_paragraph(&mut lines, info.description.as_deref());

    let mut field_lines: Vec<String> = Vec::new();
    field_lines.push(format!("**Version:** `{}`", info.version));
    if let Some(tos) = &info.terms_of_service {
        field_lines.push(format!("**Terms of Service:** <{tos}>"));
    }
    if let Some(contact) = &info.contact
        && let Some(line) = render_contact(contact)
    {
        field_lines.push(format!("**Contact:** {line}"));
    }
    if let Some(license) = &info.license {
        field_lines.push(format!("**License:** {}", render_license(license)));
    }
    push_doc_block(&mut lines, &field_lines);

    if let Some(external_docs) = &spec.external_docs {
        let header = match &external_docs.description {
            Some(desc) if !desc.trim().is_empty() => {
                format!(
                    "# External Documentation\n\n{desc}\n\nSee <{}>.",
                    external_docs.url
                )
            }
            _ => format!("# External Documentation\n\nSee <{}>.", external_docs.url),
        };
        push_doc_paragraph(&mut lines, Some(&header));
    }

    let mut doc_attrs = proc_macro2::TokenStream::new();
    for line in lines {
        let formatted = if line.is_empty() {
            String::new()
        } else {
            format!(" {line}")
        };
        doc_attrs.extend(quote! { #[doc = #formatted] });
    }
    quote! {
        #doc_attrs
        pub mod spec {}
    }
}

fn push_doc_paragraph(lines: &mut Vec<String>, text: Option<&str>) {
    let Some(text) = text else { return };
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return;
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    for line in trimmed.lines() {
        lines.push(line.to_owned());
    }
}

fn push_doc_block(lines: &mut Vec<String>, block: &[String]) {
    if block.is_empty() {
        return;
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    for entry in block {
        lines.push(format!("- {entry}"));
    }
}

fn render_contact(contact: &oas3::spec::Contact) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(name) = &contact.name {
        parts.push(name.clone());
    }
    if let Some(url) = &contact.url {
        parts.push(format!("<{url}>"));
    }
    if let Some(email) = &contact.email {
        parts.push(format!("<{email}>"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn render_license(license: &oas3::spec::License) -> String {
    let mut out = license.name.clone();
    if let Some(identifier) = &license.identifier {
        out.push_str(&format!(" (`{identifier}`)"));
    }
    if let Some(url) = &license.url {
        out.push_str(&format!(" <{url}>"));
    }
    out
}
