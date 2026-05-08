//! Mapping from OpenAPI URL path templates + HTTP method to the nested
//! Rust module path an operation lives under.
//!
//! Conventions (see `TODO.md` for the full design):
//!
//! - literal segment `foo` → `to_snake_case("foo")`
//! - path parameter `{foo}` → `by_{to_snake_case("foo")}`
//! - HTTP method → lowercase (`get`, `post`, `delete`, ...)
//! - root `/` → empty prefix; operations sit directly under the HTTP
//!   method module (`operations::get`, `operations::post`, ...)
//!
//! Examples:
//!
//! ```ignore
//! assert_eq!(
//!     mod_path("/pets/{id}", &http::Method::GET),
//!     vec!["pets", "by_id", "get"],
//! );
//! assert_eq!(mod_path("/", &http::Method::POST), vec!["post"]);
//! ```

use http::Method;

use crate::naming::to_snake_case;

/// Prefix applied to path-parameter segments. Turns `/pets/{id}` into
/// `pets::by_id::...`, which reads naturally for readers who know the
/// URL shape.
const PATH_PARAM_PREFIX: &str = "by_";

/// Produces the nested module path for an operation. Each entry is a
/// valid Rust identifier (no keyword collisions; digits-prefixed names
/// get a leading underscore through `to_snake_case`).
///
/// The HTTP method lands as the last segment, keeping attached types
/// like `Request` / `Response` grouped under their verb.
pub fn mod_path(path: &str, method: &Method) -> Vec<String> {
    let mut out: Vec<String> = path_segments(path)
        .into_iter()
        .map(|segment| match segment {
            Segment::Literal(s) => to_snake_case(s),
            Segment::Param(name) => format!("{PATH_PARAM_PREFIX}{}", to_snake_case(name)),
        })
        .collect();
    out.push(method_segment(method));
    out
}

/// One segment of a parsed path template.
enum Segment<'a> {
    Literal(&'a str),
    Param(&'a str),
}

/// Splits the path into non-empty segments. `/`, leading/trailing `/`,
/// and double-slash `//` all collapse to the empty-segment list, which
/// yields an empty prefix.
fn path_segments(path: &str) -> Vec<Segment<'_>> {
    let mut out: Vec<Segment<'_>> = Vec::new();
    for raw in path.split('/') {
        if raw.is_empty() {
            continue;
        }
        if let Some(inner) = raw.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            out.push(Segment::Param(inner));
        } else {
            out.push(Segment::Literal(raw));
        }
    }
    out
}

/// Lowercase method name. All standard HTTP methods are valid Rust
/// identifiers in lowercase, so no escape is needed.
fn method_segment(method: &Method) -> String {
    method.as_str().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get() -> Method {
        Method::GET
    }

    #[test]
    fn simple_literal_path() {
        assert_eq!(mod_path("/pets", &get()), vec!["pets", "get"]);
    }

    #[test]
    fn path_param_gets_by_prefix() {
        assert_eq!(mod_path("/pets/{id}", &get()), vec!["pets", "by_id", "get"],);
    }

    #[test]
    fn camel_case_param_snake_cased() {
        assert_eq!(
            mod_path("/pets/{petId}", &get()),
            vec!["pets", "by_pet_id", "get"],
        );
    }

    #[test]
    fn literal_camel_case_snake_cased() {
        assert_eq!(
            mod_path("/userProfiles/{id}", &get()),
            vec!["user_profiles", "by_id", "get"],
        );
    }

    #[test]
    fn root_path_yields_just_method() {
        assert_eq!(mod_path("/", &Method::POST), vec!["post"]);
    }

    #[test]
    fn trailing_slash_collapses() {
        assert_eq!(mod_path("/pets/", &get()), vec!["pets", "get"]);
    }

    #[test]
    fn multi_segment_path() {
        assert_eq!(
            mod_path("/users/{userId}/sessions", &Method::DELETE),
            vec!["users", "by_user_id", "sessions", "delete"],
        );
    }

    #[test]
    fn non_ascii_param_falls_back_gracefully() {
        // `to_snake_case` keeps alphanumerics and folds the rest into
        // underscores, so non-ASCII won't produce an invalid ident —
        // it'll produce something parseable (often `_`) but legal.
        let out = mod_path("/pets/{宠物}", &get());
        assert_eq!(out.len(), 3);
        assert!(out[1].starts_with("by_"));
    }
}
