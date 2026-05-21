//! White-box tests for the codec / Accept-header machinery in
//! [`super`]. Exercises the helpers that decide *which* response
//! variants exist, *how* they are ordered, and *what* the auto-emitted
//! `Accept` header reads — without spinning up a full
//! `Generator`.

use std::collections::BTreeMap;

use oas3::spec::MediaType;
use syn::parse_quote;

use super::{
    CodecKind, ResponseVariant, build_accept_header_value, codec_variant_suffix,
    collect_content_branches,
};

/// Default-constructs a `MediaType` with no schema / examples /
/// encoding. The codec helpers under test only look at the *map keys*
/// (the MIME strings), so the value's content is irrelevant.
fn empty_media_type() -> MediaType {
    MediaType {
        schema: None,
        examples: None,
        encoding: BTreeMap::new(),
        extensions: BTreeMap::new(),
    }
}

fn content(mimes: &[&str]) -> BTreeMap<String, MediaType> {
    mimes
        .iter()
        .map(|mime| ((*mime).to_string(), empty_media_type()))
        .collect()
}

fn variant(status: &str, codec: Option<CodecKind>, content_type: Option<&str>) -> ResponseVariant {
    let suffix = codec.map(codec_variant_suffix).unwrap_or("");
    let upper = status.to_ascii_uppercase();
    let ident_name = if suffix.is_empty() {
        format!("Status{upper}")
    } else {
        format!("Status{upper}{suffix}")
    };
    let ident = syn::Ident::new(&ident_name, proc_macro2::Span::call_site());
    let inner: Option<syn::Type> = codec.map(|_| parse_quote!(()));
    ResponseVariant {
        status: status.to_string(),
        variant_ident: ident,
        inner_type: inner,
        codec,
        content_type: content_type.map(str::to_string),
        description: None,
    }
}

#[test]
fn json_and_sse_collapse_into_distinct_codecs() {
    let mimes = content(&["application/json", "text/event-stream"]);
    let branches = collect_content_branches(&mimes);
    let codecs: Vec<_> = branches.iter().map(|(_, c)| *c).collect();
    assert_eq!(codecs, vec![CodecKind::Json, CodecKind::Sse]);
}

#[test]
fn vendor_json_collapses_to_single_json_branch() {
    // Two JSON-shaped MIMEs must yield exactly one Json branch — emitting
    // two would produce duplicate decode arms with identical bodies.
    let mimes = content(&["application/vnd.foo+json", "application/json"]);
    let branches = collect_content_branches(&mimes);
    assert_eq!(branches.len(), 1);
    let (mime, codec) = &branches[0];
    assert_eq!(*codec, CodecKind::Json);
    // `application/json` is preferred over vendor variants so the
    // generated variant ident stays stable across spec edits.
    assert_eq!(mime, "application/json");
}

#[test]
fn unknown_mimes_are_skipped() {
    let mimes = content(&["application/json", "application/x-not-real-codec"]);
    let branches = collect_content_branches(&mimes);
    assert_eq!(branches.len(), 1);
    assert_eq!(branches[0].1, CodecKind::Json);
}

#[test]
fn branches_sorted_by_codec_priority() {
    // Insertion order is alphabetical (BTreeMap), but the output must
    // follow `codec_sort_key` — Json before Sse before Text — so the
    // generated `Accept` header is deterministic.
    let mimes = content(&["text/event-stream", "text/plain", "application/json"]);
    let codecs: Vec<_> = collect_content_branches(&mimes)
        .into_iter()
        .map(|(_, c)| c)
        .collect();
    assert_eq!(
        codecs,
        vec![CodecKind::Json, CodecKind::Text, CodecKind::Sse]
    );
}

#[test]
fn accept_header_lists_every_distinct_mime_in_codec_order() {
    let variants = vec![
        variant("200", Some(CodecKind::Json), Some("application/json")),
        variant("200", Some(CodecKind::Sse), Some("text/event-stream")),
        variant("404", Some(CodecKind::Json), Some("application/json")),
    ];
    let header = build_accept_header_value(&variants).expect("header expected");
    // Json before Sse; the duplicate `application/json` from the 404
    // variant collapses into one entry.
    assert_eq!(header, "application/json, text/event-stream");
}

#[test]
fn accept_header_is_none_when_only_unit_variants() {
    // Unit responses (e.g. 204 No Content) carry no MIME — without any
    // decodeable branch the generator must omit the header entirely so
    // the wire request matches what a hand-written client would send.
    let variants = vec![variant("204", None, None)];
    assert!(build_accept_header_value(&variants).is_none());
}

#[test]
fn accept_header_dedupes_case_insensitively() {
    let variants = vec![
        variant("200", Some(CodecKind::Json), Some("Application/JSON")),
        variant("200", Some(CodecKind::Json), Some("application/json")),
    ];
    let header = build_accept_header_value(&variants).expect("header expected");
    // Either casing is acceptable, but the result must be a single entry.
    assert!(!header.contains(','));
}
