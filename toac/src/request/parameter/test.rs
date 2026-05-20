//! RFC 6570 canonical examples plus OAS-only style coverage.
//!
//! The variable definitions follow RFC 6570 §1.5 verbatim where the
//! tested style allows the input shape (this encoder operates on
//! already-stringified primitives, so the prefix-modifier `:N` and the
//! "undefined value" cases from the RFC are not exercised).

use super::*;

fn run(
    name: &str,
    value: ParameterValue<'_>,
    style: ParameterStyle,
    explode: bool,
    location: ParameterIn,
) -> String {
    let mut buf = String::new();
    let mut first = true;
    encode_parameter(&mut buf, name, value, style, explode, location, &mut first)
        .expect("encoding succeeds");
    buf
}

const HELLO: &str = "Hello World!";
const HALF: &str = "50%";
const VAR: &str = "value";
const LIST: &[&str] = &["red", "green", "blue"];
const KEYS: &[(&str, &str)] = &[("semi", ";"), ("dot", "."), ("comma", ",")];

// ---------- simple ----------

#[test]
fn simple_scalar() {
    assert_eq!(
        run(
            "var",
            ParameterValue::Scalar(VAR),
            ParameterStyle::Simple,
            false,
            ParameterIn::Path,
        ),
        "value",
    );
}

#[test]
fn simple_scalar_percent_encodes_reserved() {
    assert_eq!(
        run(
            "hello",
            ParameterValue::Scalar(HELLO),
            ParameterStyle::Simple,
            false,
            ParameterIn::Path,
        ),
        "Hello%20World%21",
    );
    assert_eq!(
        run(
            "half",
            ParameterValue::Scalar(HALF),
            ParameterStyle::Simple,
            false,
            ParameterIn::Path,
        ),
        "50%25",
    );
}

#[test]
fn simple_array_explode_and_not_match_rfc() {
    // RFC 6570: `{list}` and `{list*}` both render as `red,green,blue`.
    assert_eq!(
        run(
            "list",
            ParameterValue::Array(LIST),
            ParameterStyle::Simple,
            false,
            ParameterIn::Path,
        ),
        "red,green,blue",
    );
    assert_eq!(
        run(
            "list",
            ParameterValue::Array(LIST),
            ParameterStyle::Simple,
            true,
            ParameterIn::Path,
        ),
        "red,green,blue",
    );
}

#[test]
fn simple_object_non_explode_uses_comma_kv() {
    // RFC 6570: `{keys}` = `semi,%3B,dot,.,comma,%2C`
    assert_eq!(
        run(
            "keys",
            ParameterValue::Object(KEYS),
            ParameterStyle::Simple,
            false,
            ParameterIn::Path,
        ),
        "semi,%3B,dot,.,comma,%2C",
    );
}

#[test]
fn simple_object_explode_uses_eq_kv() {
    // RFC 6570: `{keys*}` = `semi=%3B,dot=.,comma=%2C`
    assert_eq!(
        run(
            "keys",
            ParameterValue::Object(KEYS),
            ParameterStyle::Simple,
            true,
            ParameterIn::Path,
        ),
        "semi=%3B,dot=.,comma=%2C",
    );
}

// ---------- label ----------

#[test]
fn label_scalar() {
    assert_eq!(
        run(
            "var",
            ParameterValue::Scalar(VAR),
            ParameterStyle::Label,
            false,
            ParameterIn::Path,
        ),
        ".value",
    );
}

#[test]
fn label_array_non_explode_joins_with_comma() {
    // RFC 6570 §3.2.5: `{.list}` = `.red,green,blue`
    assert_eq!(
        run(
            "list",
            ParameterValue::Array(LIST),
            ParameterStyle::Label,
            false,
            ParameterIn::Path,
        ),
        ".red,green,blue",
    );
}

#[test]
fn label_array_explode_joins_with_dot() {
    // RFC 6570 §3.2.5: `{.list*}` = `.red.green.blue`
    assert_eq!(
        run(
            "list",
            ParameterValue::Array(LIST),
            ParameterStyle::Label,
            true,
            ParameterIn::Path,
        ),
        ".red.green.blue",
    );
}

#[test]
fn label_object_non_explode() {
    // RFC 6570: `{.keys}` = `.semi,%3B,dot,.,comma,%2C`
    assert_eq!(
        run(
            "keys",
            ParameterValue::Object(KEYS),
            ParameterStyle::Label,
            false,
            ParameterIn::Path,
        ),
        ".semi,%3B,dot,.,comma,%2C",
    );
}

#[test]
fn label_object_explode() {
    // RFC 6570: `{.keys*}` = `.semi=%3B.dot=..comma=%2C`
    assert_eq!(
        run(
            "keys",
            ParameterValue::Object(KEYS),
            ParameterStyle::Label,
            true,
            ParameterIn::Path,
        ),
        ".semi=%3B.dot=..comma=%2C",
    );
}

// ---------- matrix ----------

#[test]
fn matrix_scalar() {
    assert_eq!(
        run(
            "var",
            ParameterValue::Scalar(VAR),
            ParameterStyle::Matrix,
            false,
            ParameterIn::Path,
        ),
        ";var=value",
    );
}

#[test]
fn matrix_scalar_encodes_value() {
    // RFC 6570: `{;hello}` = `;hello=Hello%20World%21`
    assert_eq!(
        run(
            "hello",
            ParameterValue::Scalar(HELLO),
            ParameterStyle::Matrix,
            false,
            ParameterIn::Path,
        ),
        ";hello=Hello%20World%21",
    );
}

#[test]
fn matrix_array_non_explode() {
    // RFC 6570: `{;list}` = `;list=red,green,blue`
    assert_eq!(
        run(
            "list",
            ParameterValue::Array(LIST),
            ParameterStyle::Matrix,
            false,
            ParameterIn::Path,
        ),
        ";list=red,green,blue",
    );
}

#[test]
fn matrix_array_explode() {
    // RFC 6570: `{;list*}` = `;list=red;list=green;list=blue`
    assert_eq!(
        run(
            "list",
            ParameterValue::Array(LIST),
            ParameterStyle::Matrix,
            true,
            ParameterIn::Path,
        ),
        ";list=red;list=green;list=blue",
    );
}

#[test]
fn matrix_object_non_explode() {
    // RFC 6570: `{;keys}` = `;keys=semi,%3B,dot,.,comma,%2C`
    assert_eq!(
        run(
            "keys",
            ParameterValue::Object(KEYS),
            ParameterStyle::Matrix,
            false,
            ParameterIn::Path,
        ),
        ";keys=semi,%3B,dot,.,comma,%2C",
    );
}

#[test]
fn matrix_object_explode() {
    // RFC 6570: `{;keys*}` = `;semi=%3B;dot=.;comma=%2C`
    assert_eq!(
        run(
            "keys",
            ParameterValue::Object(KEYS),
            ParameterStyle::Matrix,
            true,
            ParameterIn::Path,
        ),
        ";semi=%3B;dot=.;comma=%2C",
    );
}

// ---------- form ----------

#[test]
fn form_scalar() {
    // RFC 6570: `{?var}` = `?var=value`
    assert_eq!(
        run(
            "var",
            ParameterValue::Scalar(VAR),
            ParameterStyle::Form,
            false,
            ParameterIn::Query,
        ),
        "?var=value",
    );
}

#[test]
fn form_array_non_explode() {
    // RFC 6570: `{?list}` = `?list=red,green,blue`
    assert_eq!(
        run(
            "list",
            ParameterValue::Array(LIST),
            ParameterStyle::Form,
            false,
            ParameterIn::Query,
        ),
        "?list=red,green,blue",
    );
}

#[test]
fn form_array_explode() {
    // RFC 6570: `{?list*}` = `?list=red&list=green&list=blue`
    assert_eq!(
        run(
            "list",
            ParameterValue::Array(LIST),
            ParameterStyle::Form,
            true,
            ParameterIn::Query,
        ),
        "?list=red&list=green&list=blue",
    );
}

#[test]
fn form_object_non_explode() {
    // RFC 6570: `{?keys}` = `?keys=semi,%3B,dot,.,comma,%2C`
    assert_eq!(
        run(
            "keys",
            ParameterValue::Object(KEYS),
            ParameterStyle::Form,
            false,
            ParameterIn::Query,
        ),
        "?keys=semi,%3B,dot,.,comma,%2C",
    );
}

#[test]
fn form_object_explode() {
    // RFC 6570: `{?keys*}` = `?semi=%3B&dot=.&comma=%2C`
    assert_eq!(
        run(
            "keys",
            ParameterValue::Object(KEYS),
            ParameterStyle::Form,
            true,
            ParameterIn::Query,
        ),
        "?semi=%3B&dot=.&comma=%2C",
    );
}

#[test]
fn form_chains_with_ampersand() {
    let mut buf = String::new();
    let mut first = true;
    encode_parameter(
        &mut buf,
        "a",
        ParameterValue::Scalar("1"),
        ParameterStyle::Form,
        false,
        ParameterIn::Query,
        &mut first,
    )
    .expect("first ok");
    encode_parameter(
        &mut buf,
        "b",
        ParameterValue::Scalar("2"),
        ParameterStyle::Form,
        false,
        ParameterIn::Query,
        &mut first,
    )
    .expect("second ok");
    assert_eq!(buf, "?a=1&b=2");
    assert!(!first);
}

#[test]
fn form_empty_array_is_noop() {
    let mut buf = String::new();
    let mut first = true;
    encode_parameter(
        &mut buf,
        "list",
        ParameterValue::Array(&[]),
        ParameterStyle::Form,
        true,
        ParameterIn::Query,
        &mut first,
    )
    .expect("ok");
    assert!(buf.is_empty());
    assert!(first, "first must remain set when nothing was emitted");
}

// ---------- spaceDelimited / pipeDelimited ----------

#[test]
fn space_delimited_array_non_explode() {
    // OAS: `?id=3%204%205`
    assert_eq!(
        run(
            "id",
            ParameterValue::Array(&["3", "4", "5"]),
            ParameterStyle::SpaceDelimited,
            false,
            ParameterIn::Query,
        ),
        "?id=3%204%205",
    );
}

#[test]
fn pipe_delimited_array_non_explode() {
    // OAS: `?id=3|4|5`
    assert_eq!(
        run(
            "id",
            ParameterValue::Array(&["3", "4", "5"]),
            ParameterStyle::PipeDelimited,
            false,
            ParameterIn::Query,
        ),
        "?id=3|4|5",
    );
}

#[test]
fn pipe_delimited_array_explode_falls_back_to_form() {
    assert_eq!(
        run(
            "id",
            ParameterValue::Array(&["3", "4", "5"]),
            ParameterStyle::PipeDelimited,
            true,
            ParameterIn::Query,
        ),
        "?id=3&id=4&id=5",
    );
}

#[test]
fn space_delimited_rejects_scalar() {
    let mut buf = String::new();
    let mut first = true;
    let err = encode_parameter(
        &mut buf,
        "id",
        ParameterValue::Scalar("3"),
        ParameterStyle::SpaceDelimited,
        false,
        ParameterIn::Query,
        &mut first,
    )
    .expect_err("scalar is not a valid shape");
    assert!(matches!(err, EncodeError::InvalidShape { .. }));
}

// ---------- deepObject ----------

#[test]
fn deep_object_explode_object() {
    // OAS: `?color[R]=100&color[G]=200&color[B]=150`
    assert_eq!(
        run(
            "color",
            ParameterValue::Object(&[("R", "100"), ("G", "200"), ("B", "150")]),
            ParameterStyle::DeepObject,
            true,
            ParameterIn::Query,
        ),
        "?color[R]=100&color[G]=200&color[B]=150",
    );
}

#[test]
fn deep_object_rejects_array() {
    let mut buf = String::new();
    let mut first = true;
    let err = encode_parameter(
        &mut buf,
        "color",
        ParameterValue::Array(&["a", "b"]),
        ParameterStyle::DeepObject,
        true,
        ParameterIn::Query,
        &mut first,
    )
    .expect_err("array is not a valid shape");
    assert!(matches!(err, EncodeError::InvalidShape { .. }));
}

#[test]
fn deep_object_rejects_non_explode() {
    let mut buf = String::new();
    let mut first = true;
    let err = encode_parameter(
        &mut buf,
        "color",
        ParameterValue::Object(&[("R", "100")]),
        ParameterStyle::DeepObject,
        false,
        ParameterIn::Query,
        &mut first,
    )
    .expect_err("non-explode is undefined");
    assert!(matches!(err, EncodeError::UndefinedCombination { .. }));
}

// ---------- location validation ----------

#[test]
fn matrix_outside_path_rejected() {
    let mut buf = String::new();
    let mut first = true;
    let err = encode_parameter(
        &mut buf,
        "id",
        ParameterValue::Scalar("1"),
        ParameterStyle::Matrix,
        false,
        ParameterIn::Query,
        &mut first,
    )
    .expect_err("matrix is path-only");
    assert!(matches!(err, EncodeError::InvalidLocation { .. }));
}

#[test]
fn deep_object_outside_query_rejected() {
    let mut buf = String::new();
    let mut first = true;
    let err = encode_parameter(
        &mut buf,
        "id",
        ParameterValue::Object(&[("a", "1")]),
        ParameterStyle::DeepObject,
        true,
        ParameterIn::Path,
        &mut first,
    )
    .expect_err("deepObject is query-only");
    assert!(matches!(err, EncodeError::InvalidLocation { .. }));
}

#[test]
fn form_in_cookie_allowed() {
    // OAS: form is also legal for cookies.
    assert_eq!(
        run(
            "session",
            ParameterValue::Scalar("abc"),
            ParameterStyle::Form,
            false,
            ParameterIn::Cookie,
        ),
        "?session=abc",
    );
}
