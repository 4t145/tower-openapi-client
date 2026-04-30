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

/// Returns an inner doc attribute (`#![doc = "..."]`) suitable for file-level
/// documentation. Used to project an OpenAPI `externalDocs` entry into the
/// top of the generated module.
pub fn outer_file_docs(description: Option<&str>, url: Option<&str>) -> proc_macro2::TokenStream {
    let mut tokens = proc_macro2::TokenStream::new();
    if let Some(description) = description {
        for line in description.lines() {
            let formatted = format!(" {line}");
            tokens.extend(quote! { #![doc = #formatted] });
        }
    }
    if let Some(url) = url {
        let formatted = format!(" See <{url}>.");
        tokens.extend(quote! { #![doc = #formatted] });
    }
    tokens
}
