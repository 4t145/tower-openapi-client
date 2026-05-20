//! OpenAPI parameter serialization, RFC 6570-style.
//!
//! OpenAPI 3.x picks parameter wire form from a `(style, explode)` pair —
//! see <https://spec.openapis.org/oas/v3.1.1#style-values>. The four
//! original styles (`matrix`, `label`, `form`, `simple`) come straight
//! from RFC 6570 URI Templates, which already specifies how primitives,
//! arrays, and objects expand under each prefix operator. The three
//! query-only OAS additions (`spaceDelimited`, `pipeDelimited`,
//! `deepObject`) are layered on top with their own ad-hoc rules.
//!
//! This module exposes [`encode_parameter`] — generated code calls it
//! once per parameter, passing the value as a [`ParameterValue`] view
//! over already-stringified field data. Cross-parameter state (whether
//! we've already emitted the leading `?` for a query string) lives in
//! the caller through the `first` flag.
//!
//! Percent-encoding follows RFC 3986: every byte not in the relevant
//! "unreserved + allowed" set gets `%HH`-escaped. The allowed set
//! depends on the style — RFC 6570 defines two: `U` (unreserved only,
//! used by simple/label/form/matrix) and `U+R` (unreserved + reserved,
//! used by the `+`-operator forms, which OAS doesn't currently surface).
//! All emitted styles use the `U` set.

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

/// Where the parameter is carried on the wire.
///
/// Drives style/explode validation: e.g. `matrix` and `label` are only
/// legal in `path`, `form` only in `query` and `cookie`, and the OAS
/// extensions (`spaceDelimited`, `pipeDelimited`, `deepObject`) only in
/// `query`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterIn {
    Query,
    Header,
    Path,
    Cookie,
}

/// Wire-form style chosen for a parameter.
///
/// Mirrors `Parameter.style` from the OpenAPI spec verbatim. The four
/// RFC 6570 styles are reused as-is; the three OAS-only extensions
/// follow the loose definitions in the spec text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterStyle {
    /// Path-style: leading `;`, optional name. RFC 6570 `{;var}`.
    Matrix,
    /// Label-style: leading `.`. RFC 6570 `{.var}`.
    Label,
    /// Form-style: `name=value` with `?` / `&` separators chosen by the
    /// caller. RFC 6570 `{?var}` / `{&var}`.
    Form,
    /// Simple-style: comma-joined, no name. RFC 6570 `{var}`.
    Simple,
    /// Query-only: array elements joined by `%20`. OAS extension.
    SpaceDelimited,
    /// Query-only: array elements joined by `|`. OAS extension.
    PipeDelimited,
    /// Query-only: object properties projected as `key[prop]=value`.
    /// OAS extension; only `explode = true` is well-defined.
    DeepObject,
}

/// Already-stringified parameter value as seen by the encoder.
///
/// The borrow form lets generated code pass slice views without an
/// extra round of allocation. OAS restricts query-bound array items
/// and object property values to primitives, so the inner type is
/// `&str` rather than a recursive enum.
#[derive(Debug, Clone, Copy)]
pub enum ParameterValue<'a> {
    /// A single value rendered through `Display` upstream.
    Scalar(&'a str),
    /// Array of primitives.
    Array(&'a [&'a str]),
    /// Object whose property values are primitives.
    Object(&'a [(&'a str, &'a str)]),
}

/// Reasons [`encode_parameter`] declines to emit a value.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    /// The `(style, location)` pair is forbidden by the OpenAPI spec
    /// (e.g. `matrix` outside `path`, `deepObject` outside `query`).
    #[error("style {style:?} is not valid for location {location:?}")]
    InvalidLocation {
        style: ParameterStyle,
        location: ParameterIn,
    },

    /// The `(style, value-shape)` pair is forbidden by the OpenAPI spec
    /// (e.g. `spaceDelimited` against a primitive, `deepObject` against
    /// an array).
    #[error("style {style:?} cannot encode {shape}")]
    InvalidShape {
        style: ParameterStyle,
        shape: &'static str,
    },

    /// The combination is permitted by the OAS grammar but the spec
    /// leaves the wire form undefined (e.g. `deepObject` with
    /// `explode = false`).
    #[error("style {style:?} with explode={explode} is undefined for {shape}")]
    UndefinedCombination {
        style: ParameterStyle,
        explode: bool,
        shape: &'static str,
    },
}

/// Appends `name`/`value` to `dst` per OAS `(style, explode)` rules.
///
/// `first` tracks whether this is the first emitted parameter for its
/// surrounding container — the form-style picks `?` vs `&` based on
/// it. Other styles emit a constant prefix (`;` for matrix, `.` for
/// label) regardless. The flag is updated on success so chained calls
/// just thread the same `&mut bool`.
///
/// # Errors
///
/// Returns [`EncodeError::InvalidLocation`] when the `(style, location)`
/// pair is illegal, [`EncodeError::InvalidShape`] when the `(style,
/// value-shape)` pair is illegal, and
/// [`EncodeError::UndefinedCombination`] for spec-undefined cases like
/// `deepObject` + `explode = false`.
pub fn encode_parameter(
    dst: &mut String,
    name: &str,
    value: ParameterValue<'_>,
    style: ParameterStyle,
    explode: bool,
    location: ParameterIn,
    first: &mut bool,
) -> Result<(), EncodeError> {
    validate_location(style, location)?;
    match style {
        ParameterStyle::Matrix => encode_matrix(dst, name, value, explode),
        ParameterStyle::Label => encode_label(dst, name, value, explode),
        ParameterStyle::Form => encode_form(dst, name, value, explode, first),
        ParameterStyle::Simple => encode_simple(dst, name, value, explode),
        ParameterStyle::SpaceDelimited => {
            encode_delimited(dst, name, value, explode, first, "%20", style)
        }
        ParameterStyle::PipeDelimited => {
            encode_delimited(dst, name, value, explode, first, "|", style)
        }
        ParameterStyle::DeepObject => encode_deep_object(dst, name, value, explode, first),
    }
}

/// RFC 3986 unreserved set: `A-Z / a-z / 0-9 / "-" / "." / "_" / "~"`.
/// Every other byte (including all the spec's "reserved" delimiters)
/// gets percent-escaped. Same set RFC 6570 calls `unreserved`.
const UNRESERVED: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

fn pct(out: &mut String, s: &str) {
    for chunk in utf8_percent_encode(s, UNRESERVED) {
        out.push_str(chunk);
    }
}

fn validate_location(style: ParameterStyle, location: ParameterIn) -> Result<(), EncodeError> {
    let ok = match style {
        ParameterStyle::Matrix | ParameterStyle::Label => location == ParameterIn::Path,
        ParameterStyle::Form => {
            matches!(location, ParameterIn::Query | ParameterIn::Cookie)
        }
        ParameterStyle::Simple => {
            matches!(location, ParameterIn::Path | ParameterIn::Header)
        }
        ParameterStyle::SpaceDelimited
        | ParameterStyle::PipeDelimited
        | ParameterStyle::DeepObject => location == ParameterIn::Query,
    };
    if ok {
        Ok(())
    } else {
        Err(EncodeError::InvalidLocation { style, location })
    }
}

fn shape_str(value: &ParameterValue<'_>) -> &'static str {
    match value {
        ParameterValue::Scalar(_) => "scalar",
        ParameterValue::Array(_) => "array",
        ParameterValue::Object(_) => "object",
    }
}

// ---------- matrix (`{;var}`) ----------
//
// Always prefixed with `;`. Empty arrays / empty objects emit nothing
// after the prefix per RFC 6570 §3.2.7.

fn encode_matrix(
    dst: &mut String,
    name: &str,
    value: ParameterValue<'_>,
    explode: bool,
) -> Result<(), EncodeError> {
    match value {
        ParameterValue::Scalar(v) => {
            dst.push(';');
            pct(dst, name);
            dst.push('=');
            pct(dst, v);
        }
        ParameterValue::Array(items) => {
            if items.is_empty() {
                return Ok(());
            }
            if explode {
                for item in items {
                    dst.push(';');
                    pct(dst, name);
                    dst.push('=');
                    pct(dst, item);
                }
            } else {
                dst.push(';');
                pct(dst, name);
                dst.push('=');
                join_pct(dst, items.iter().copied(), ",");
            }
        }
        ParameterValue::Object(props) => {
            if props.is_empty() {
                return Ok(());
            }
            if explode {
                for (k, v) in props {
                    dst.push(';');
                    pct(dst, k);
                    dst.push('=');
                    pct(dst, v);
                }
            } else {
                dst.push(';');
                pct(dst, name);
                dst.push('=');
                join_pct_pairs(dst, props.iter().copied(), ",", ",");
            }
        }
    }
    Ok(())
}

// ---------- label (`{.var}`) ----------
//
// Always prefixed with `.`. RFC 6570 §3.2.5: explode joiner is `.` for
// arrays, `.` between properties for objects (with `=` between key and
// value). Non-explode joins everything with `,`.

fn encode_label(
    dst: &mut String,
    _name: &str,
    value: ParameterValue<'_>,
    explode: bool,
) -> Result<(), EncodeError> {
    match value {
        ParameterValue::Scalar(v) => {
            dst.push('.');
            pct(dst, v);
        }
        ParameterValue::Array(items) => {
            if items.is_empty() {
                return Ok(());
            }
            dst.push('.');
            // RFC 6570 §3.2.5: label-explode arrays join with `.`,
            // non-explode joins with `,`.
            let sep = if explode { "." } else { "," };
            join_pct(dst, items.iter().copied(), sep);
        }
        ParameterValue::Object(props) => {
            if props.is_empty() {
                return Ok(());
            }
            dst.push('.');
            if explode {
                join_pct_pairs(dst, props.iter().copied(), "=", ".");
            } else {
                join_pct_pairs(dst, props.iter().copied(), ",", ",");
            }
        }
    }
    Ok(())
}

// ---------- form (`{?var}` / `{&var}`) ----------
//
// First parameter in the query string is prefixed `?`, subsequent ones
// `&`. The encoder mutates `*first` after emitting so callers don't
// have to track the toggle themselves.

fn encode_form(
    dst: &mut String,
    name: &str,
    value: ParameterValue<'_>,
    explode: bool,
    first: &mut bool,
) -> Result<(), EncodeError> {
    let prefix = |dst: &mut String, first: &mut bool| {
        dst.push(if *first { '?' } else { '&' });
        *first = false;
    };
    match value {
        ParameterValue::Scalar(v) => {
            prefix(dst, first);
            pct(dst, name);
            dst.push('=');
            pct(dst, v);
        }
        ParameterValue::Array(items) => {
            if items.is_empty() {
                return Ok(());
            }
            if explode {
                for item in items {
                    prefix(dst, first);
                    pct(dst, name);
                    dst.push('=');
                    pct(dst, item);
                }
            } else {
                prefix(dst, first);
                pct(dst, name);
                dst.push('=');
                join_pct(dst, items.iter().copied(), ",");
            }
        }
        ParameterValue::Object(props) => {
            if props.is_empty() {
                return Ok(());
            }
            if explode {
                for (k, v) in props {
                    prefix(dst, first);
                    pct(dst, k);
                    dst.push('=');
                    pct(dst, v);
                }
            } else {
                prefix(dst, first);
                pct(dst, name);
                dst.push('=');
                join_pct_pairs(dst, props.iter().copied(), ",", ",");
            }
        }
    }
    Ok(())
}

// ---------- simple (`{var}`) ----------
//
// No prefix and no name on the wire — generated code uses this from
// path templates after substituting placeholders. Empty array / object
// renders to nothing.

fn encode_simple(
    dst: &mut String,
    _name: &str,
    value: ParameterValue<'_>,
    explode: bool,
) -> Result<(), EncodeError> {
    match value {
        ParameterValue::Scalar(v) => pct(dst, v),
        ParameterValue::Array(items) => join_pct(dst, items.iter().copied(), ","),
        ParameterValue::Object(props) => {
            let kv_sep = if explode { "=" } else { "," };
            join_pct_pairs(dst, props.iter().copied(), kv_sep, ",");
        }
    }
    Ok(())
}

// ---------- spaceDelimited / pipeDelimited ----------
//
// OAS additions for arrays in `query`. Non-explode joins items with
// the style's literal delimiter (space → `%20`, pipe → `|`). Explode
// degenerates to the form-explode behaviour because the spec says it
// "behaves the same as form" once each item gets its own
// `name=value`. Object shapes are not defined; primitive values are
// rejected because the OAS table only covers arrays.

fn encode_delimited(
    dst: &mut String,
    name: &str,
    value: ParameterValue<'_>,
    explode: bool,
    first: &mut bool,
    join: &str,
    style: ParameterStyle,
) -> Result<(), EncodeError> {
    match value {
        ParameterValue::Array(items) => {
            if items.is_empty() {
                return Ok(());
            }
            if explode {
                for item in items {
                    dst.push(if *first { '?' } else { '&' });
                    *first = false;
                    pct(dst, name);
                    dst.push('=');
                    pct(dst, item);
                }
            } else {
                dst.push(if *first { '?' } else { '&' });
                *first = false;
                pct(dst, name);
                dst.push('=');
                join_pct(dst, items.iter().copied(), join);
            }
            Ok(())
        }
        other => Err(EncodeError::InvalidShape {
            style,
            shape: shape_str(&other),
        }),
    }
}

// ---------- deepObject ----------
//
// `?key[prop1]=v1&key[prop2]=v2`. Spec only nails down explode=true
// for objects — every other combination is rejected so generated code
// surfaces it as a hard error rather than silently picking a form.

fn encode_deep_object(
    dst: &mut String,
    name: &str,
    value: ParameterValue<'_>,
    explode: bool,
    first: &mut bool,
) -> Result<(), EncodeError> {
    let ParameterValue::Object(props) = value else {
        return Err(EncodeError::InvalidShape {
            style: ParameterStyle::DeepObject,
            shape: shape_str(&value),
        });
    };
    if !explode {
        return Err(EncodeError::UndefinedCombination {
            style: ParameterStyle::DeepObject,
            explode,
            shape: "object",
        });
    }
    for (k, v) in props {
        dst.push(if *first { '?' } else { '&' });
        *first = false;
        pct(dst, name);
        dst.push('[');
        pct(dst, k);
        dst.push(']');
        dst.push('=');
        pct(dst, v);
    }
    Ok(())
}

// ---------- helpers ----------

fn join_pct<'a, I: IntoIterator<Item = &'a str>>(out: &mut String, items: I, sep: &str) {
    let mut first = true;
    for item in items {
        if !first {
            out.push_str(sep);
        }
        first = false;
        pct(out, item);
    }
}

fn join_pct_pairs<'a, I: IntoIterator<Item = (&'a str, &'a str)>>(
    out: &mut String,
    pairs: I,
    kv_sep: &str,
    pair_sep: &str,
) {
    let mut first = true;
    for (k, v) in pairs {
        if !first {
            out.push_str(pair_sep);
        }
        first = false;
        pct(out, k);
        out.push_str(kv_sep);
        pct(out, v);
    }
}

/// Failure raised by [`encode_serialized`].
///
/// Splits the two distinct failure paths so callers can act on them
/// without string-matching: shape conversion (the value normalises to
/// JSON but doesn't fit the OAS array-of-primitives / object-of-primitives
/// constraint), encoding (the encoder rejects the value for the picked
/// `(style, location)` combination), and `Serialize::serialize` itself.
#[derive(Debug, thiserror::Error)]
pub enum SerializeEncodeError {
    /// `Serialize::serialize` returned an error before normalisation.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// The serialised JSON shape is incompatible with OAS parameter
    /// rules (e.g. an array element that's itself an object, or a top-
    /// level value that's neither scalar / array / object).
    #[error("value shape is not encodable as a parameter: {reason}")]
    UnsupportedShape {
        /// Brief description of the shape problem.
        reason: &'static str,
    },

    /// The encoder rejected the value (style/location/explode mismatch).
    #[error(transparent)]
    Encode(#[from] EncodeError),
}

/// Serialises `value` and appends it to `dst` per OAS `(style, explode)`
/// rules. Convenience wrapper around [`encode_parameter`] that lets
/// generated code stay agnostic about whether a parameter is a primitive,
/// an array, or an object — the runtime inspects the JSON form and
/// picks the right [`ParameterValue`] variant.
///
/// Cost: one `serde_json::Value` allocation per parameter, plus
/// per-element string conversion. Both stay constant per call and away
/// from the hot path on requests without query parameters (the generator
/// only emits this call when the operation declares one).
///
/// # Errors
///
/// Returns [`SerializeEncodeError::Serde`] if `Serialize::serialize`
/// fails, [`SerializeEncodeError::UnsupportedShape`] when the JSON form
/// can't be projected into a [`ParameterValue`] (e.g. nested arrays or
/// non-string property keys at the top level), and
/// [`SerializeEncodeError::Encode`] for the encoder's own validation
/// failures.
pub fn encode_serialized<T>(
    dst: &mut String,
    name: &str,
    value: &T,
    style: ParameterStyle,
    explode: bool,
    location: ParameterIn,
    first: &mut bool,
) -> Result<(), SerializeEncodeError>
where
    T: serde::Serialize + ?Sized,
{
    let json = serde_json::to_value(value)?;
    encode_serialized_value(dst, name, &json, style, explode, location, first)
}

/// Same as [`encode_serialized`] but starts from an already-built
/// [`serde_json::Value`]. Useful when generated code has projected the
/// field through some other adapter (`HashMap`, `BTreeMap`, …) before
/// reaching the encoder.
///
/// # Errors
///
/// Same conditions as [`encode_serialized`], minus the `Serde` variant
/// (the value is already built).
pub fn encode_serialized_value(
    dst: &mut String,
    name: &str,
    json: &serde_json::Value,
    style: ParameterStyle,
    explode: bool,
    location: ParameterIn,
    first: &mut bool,
) -> Result<(), SerializeEncodeError> {
    use serde_json::Value;

    // OAS treats `null` parameter values the same as "not present" —
    // the field renders to nothing and `first` stays untouched. This
    // matches what generated code already does for `Option::None`.
    if json.is_null() {
        return Ok(());
    }

    match json {
        Value::Null => Ok(()),
        Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            let scalar = scalar_to_string(json);
            encode_parameter(
                dst,
                name,
                ParameterValue::Scalar(&scalar),
                style,
                explode,
                location,
                first,
            )?;
            Ok(())
        }
        Value::Array(items) => {
            let mut owned: Vec<String> = Vec::with_capacity(items.len());
            for item in items {
                if !is_primitive(item) {
                    return Err(SerializeEncodeError::UnsupportedShape {
                        reason: "array element is not a primitive",
                    });
                }
                if item.is_null() {
                    // OAS doesn't define a wire form for "array element
                    // is null"; skip rather than guess.
                    continue;
                }
                owned.push(scalar_to_string(item));
            }
            let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
            encode_parameter(
                dst,
                name,
                ParameterValue::Array(&refs),
                style,
                explode,
                location,
                first,
            )?;
            Ok(())
        }
        Value::Object(map) => {
            let mut owned: Vec<(String, String)> = Vec::with_capacity(map.len());
            for (k, v) in map {
                if !is_primitive(v) {
                    return Err(SerializeEncodeError::UnsupportedShape {
                        reason: "object property value is not a primitive",
                    });
                }
                if v.is_null() {
                    continue;
                }
                owned.push((k.clone(), scalar_to_string(v)));
            }
            let refs: Vec<(&str, &str)> = owned
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            encode_parameter(
                dst,
                name,
                ParameterValue::Object(&refs),
                style,
                explode,
                location,
                first,
            )?;
            Ok(())
        }
    }
}

fn is_primitive(v: &serde_json::Value) -> bool {
    matches!(
        v,
        serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::String(_)
    )
}

/// Renders a JSON primitive as the wire string. Strings are passed
/// through verbatim (no extra JSON quoting); booleans / numbers go
/// through their `Display` form. Caller guarantees the value is a
/// primitive — `Null`, `Array`, and `Object` would be wrong here.
fn scalar_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        // Should be unreachable per caller guarantee, but render through
        // the canonical JSON form rather than panicking — matches the
        // "do not let codegen issues turn into client crashes" stance.
        _ => v.to_string(),
    }
}

#[cfg(test)]
mod test;
