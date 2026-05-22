//! Identifier normalization utilities.
//!
//! Translates arbitrary strings from an OpenAPI spec into valid Rust idents,
//! applying the standard casing conventions (PascalCase for types, snake_case
//! for fields) and quoting reserved words as raw identifiers when possible.

use proc_macro2::Span;

/// Rust keywords the language forbids from appearing even as raw idents
/// (see the Rust Reference, "Raw identifiers"). These get a trailing
/// underscore instead. `_` is the wildcard pattern, not an ident, so it
/// is in the same bucket — `r#_` is rejected by the parser.
const RAW_IDENT_FORBIDDEN: &[&str] = &["crate", "self", "Self", "super", "_"];

/// Fallback suffix appended to otherwise-unspellable idents.
const IDENT_SUFFIX_FALLBACK: &str = "_";

/// Maps operator-like ASCII symbols to English word equivalents. Mirrors
/// the table `openapi-generator` uses in `DefaultCodegen.getSymbolName`,
/// which is the de-facto standard across OpenAPI / Swagger toolchains.
///
/// Without this mapping, distinct enum members like `"+1"` and `"-1"`
/// collapse to the same Rust identifier (`_1`), producing duplicate
/// variants. Returns `None` for characters that aren't in the table —
/// they keep their existing "treat as a word boundary" behaviour.
fn symbol_word(ch: char) -> Option<&'static str> {
    let word = match ch {
        '+' => "Plus",
        '-' => "Minus",
        '*' => "Star",
        '/' => "Slash",
        '\\' => "Backslash",
        '=' => "Equal",
        '>' => "GreaterThan",
        '<' => "LessThan",
        '!' => "Bang",
        '&' => "And",
        '|' => "Or",
        '^' => "Caret",
        '%' => "Percent",
        '@' => "At",
        '#' => "Hash",
        '$' => "Dollar",
        '?' => "Question",
        '~' => "Tilde",
        _ => return None,
    };
    Some(word)
}

/// Sentinel segment prepended to identifiers derived from purely
/// symbolic inputs (e.g. `"*"`). Without it the operator word can
/// shadow an unrelated literal — GitHub's webhook event enum, for
/// instance, declares both `"*"` and `"star"` as members, and a naive
/// substitution maps both to `Star`.
///
/// `Sym` is alphabetic so `to_snake_case` keeps it as a single segment
/// and Rust accepts it directly as part of an ident.
const SYMBOL_ONLY_PREFIX: &str = "Sym";

/// Pre-pass that swaps operator-like symbols for their English word
/// names so they survive downstream case conversion.
///
/// Heuristic: only substitute when the symbol *carries semantic weight*,
/// i.e. it's not just a kebab- or camel-style word separator. A symbol
/// is treated as a separator (kept as-is, so the existing "non-alphanum
/// → underscore" path in [`to_snake_case`] handles it) iff at least one
/// neighbour is a letter — that captures `sandbox-detail`, `pets@admin`
/// and similar conventional cases. Otherwise the symbol is replaced,
/// preserving the wire-level distinction between `"+1"` and `"-1"`,
/// `"reactions-+1"` and `"reactions--1"`, and so on.
///
/// Replacement words are surrounded with spaces so [`to_snake_case`]
/// treats them as their own segment. Pure-symbol inputs (no ASCII
/// letters) additionally get [`SYMBOL_ONLY_PREFIX`] so the result can't
/// clash with a homonymous literal value also declared in the spec.
fn replace_symbols(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let has_alphanum = chars.iter().any(char::is_ascii_alphanumeric);
    let mut out = String::with_capacity(input.len());
    if !has_alphanum && chars.iter().any(|c| symbol_word(*c).is_some()) {
        out.push_str(SYMBOL_ONLY_PREFIX);
        out.push(' ');
    }
    for (i, &ch) in chars.iter().enumerate() {
        let Some(word) = symbol_word(ch) else {
            out.push(ch);
            continue;
        };
        let left = i.checked_sub(1).map(|j| chars[j]);
        let right = chars.get(i + 1).copied();
        let adjacent_to_letter = matches!(left, Some(c) if c.is_ascii_alphabetic())
            || matches!(right, Some(c) if c.is_ascii_alphabetic());
        if adjacent_to_letter {
            out.push(ch);
        } else {
            out.push(' ');
            out.push_str(word);
            out.push(' ');
        }
    }
    out
}

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
/// Operator-like symbols that don't sit between two letters are mapped
/// to their English word names by [`replace_symbols`] first, so values
/// like `"+1"` and `"-1"` keep distinct identifiers (`plus_1` /
/// `minus_1`) instead of both collapsing to `_1`.
pub fn to_snake_case(input: &str) -> String {
    let mapped = replace_symbols(input);
    let mut out = String::with_capacity(mapped.len() + 4);
    let chars: Vec<char> = mapped.chars().collect();

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
    fn pascal_case_preserves_symbol_semantics() {
        // `+1` and `-1` would both collapse to `_1` without the symbol
        // pre-pass; the operator words keep them distinct.
        assert_eq!(to_pascal_case("+1"), "Plus1");
        assert_eq!(to_pascal_case("-1"), "Minus1");
        assert_eq!(to_pascal_case("reactions-+1"), "ReactionsPlus1");
        assert_eq!(to_pascal_case("reactions--1"), "ReactionsMinus1");
    }

    #[test]
    fn snake_case_preserves_symbol_semantics() {
        assert_eq!(to_snake_case("+1"), "plus_1");
        assert_eq!(to_snake_case("-1"), "minus_1");
    }

    #[test]
    fn pure_symbol_inputs_get_distinguishing_prefix() {
        // GitHub's webhook event enum declares both `"*"` and `"star"`.
        // Without the prefix, both would map to `Star` and collide.
        assert_eq!(to_pascal_case("*"), "SymStar");
        assert_eq!(to_pascal_case("star"), "Star");
    }

    #[test]
    fn separator_dash_between_words_stays_a_separator() {
        // `-` between two words is a conventional kebab-case separator
        // and must keep the existing splitting behaviour. Only standalone
        // `-` (e.g. the one in `-1`) gets mapped to `Minus`.
        assert_eq!(to_pascal_case("foo-bar"), "FooBar");
        assert_eq!(to_snake_case("foo-bar"), "foo_bar");
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
