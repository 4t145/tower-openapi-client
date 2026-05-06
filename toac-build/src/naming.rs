//! Identifier normalization utilities.
//!
//! Translates arbitrary strings from an OpenAPI spec into valid Rust idents,
//! applying the standard casing conventions (PascalCase for types, snake_case
//! for fields) and quoting reserved words as raw identifiers when possible.

use proc_macro2::Span;

/// Rust keywords the language forbids from appearing even as raw idents
/// (see the Rust Reference, "Raw identifiers"). These get a trailing
/// underscore instead.
const RAW_IDENT_FORBIDDEN: &[&str] = &["crate", "self", "Self", "super"];

/// Fallback suffix appended to otherwise-unspellable idents.
const IDENT_SUFFIX_FALLBACK: &str = "_";

/// Turns an arbitrary string into a PascalCase Rust identifier.
///
/// The transformation is: normalise into snake_case first (which splits
/// existing humps and uppercase runs like `PUT` → `put`, `APIKey` →
/// `api_key`), then capitalise each `_`-delimited segment. This gives a
/// stable mapping: `getPet`, `get_pet`, `GET pet`, `GetPet` all land on
/// `GetPet`.
pub fn to_pascal_case(input: &str) -> String {
    let snake = to_snake_case(input);
    let mut out = String::with_capacity(snake.len());
    let mut upper_next = true;

    for ch in snake.chars() {
        if ch == '_' {
            upper_next = true;
            continue;
        }
        if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }

    if out.is_empty() {
        return IDENT_SUFFIX_FALLBACK.to_owned();
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// Turns an arbitrary string into a snake_case Rust identifier.
///
/// Camel/Pascal humps split on the leading uppercase letter; runs of
/// uppercase letters are treated as a single word (e.g. `APIKey` → `api_key`).
pub fn to_snake_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 4);
    let chars: Vec<char> = input.chars().collect();

    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_ascii_alphanumeric() {
            let needs_underscore = ch.is_ascii_uppercase()
                && !out.is_empty()
                && !out.ends_with('_')
                && (chars[i - 1].is_ascii_lowercase()
                    || chars[i - 1].is_ascii_digit()
                    || chars
                        .get(i + 1)
                        .is_some_and(|next| next.is_ascii_lowercase()));
            if needs_underscore {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }

    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        return IDENT_SUFFIX_FALLBACK.to_owned();
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// Creates a Rust identifier from `raw`, using the `r#` escape for keywords
/// that support it and appending `_` for those that don't.
pub fn make_ident(raw: &str) -> syn::Ident {
    if RAW_IDENT_FORBIDDEN.contains(&raw) {
        let fallback = format!("{raw}{IDENT_SUFFIX_FALLBACK}");
        return syn::Ident::new(&fallback, Span::call_site());
    }
    match syn::parse_str::<syn::Ident>(raw) {
        Ok(ident) => ident,
        Err(_) => syn::Ident::new_raw(raw, Span::call_site()),
    }
}

/// PascalCase type identifier.
pub fn type_ident(name: &str) -> syn::Ident {
    make_ident(&to_pascal_case(name))
}

/// snake_case field identifier.
pub fn field_ident(name: &str) -> syn::Ident {
    make_ident(&to_snake_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_case_handles_separators() {
        assert_eq!(to_pascal_case("sandbox_detail"), "SandboxDetail");
        assert_eq!(to_pascal_case("sandbox-detail"), "SandboxDetail");
        assert_eq!(to_pascal_case("SandboxDetail"), "SandboxDetail");
        assert_eq!(to_pascal_case("400"), "_400");
    }

    #[test]
    fn snake_case_splits_humps() {
        assert_eq!(to_snake_case("sandboxID"), "sandbox_id");
        assert_eq!(to_snake_case("APIKey"), "api_key");
        assert_eq!(to_snake_case("startedAt"), "started_at");
        assert_eq!(to_snake_case("HTTPServer"), "http_server");
        assert_eq!(
            to_snake_case("allow_internet_access"),
            "allow_internet_access"
        );
    }

    #[test]
    fn keywords_get_raw_escape() {
        let ident = make_ident("type");
        assert_eq!(ident.to_string(), "r#type");
    }

    #[test]
    fn forbidden_keywords_get_suffix() {
        let ident = make_ident("self");
        assert_eq!(ident.to_string(), "self_");
    }
}
