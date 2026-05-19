//! Unit tests for the non-JSON body codecs (`text`, `octet`, `form`,
//! `multipart`). JSON has its own coverage in the runtime-surface and
//! ApiClient tests.

#![allow(clippy::manual_async_fn)]

use ::bytes::Bytes;
use ::http_body_util::Full;
use ::toac::body::{
    Body,
    codec::{
        BodyContentType, BodyDecoder, BodyEncoder,
        form::FormEncoder,
        multipart::{MultipartEncoder, MultipartForm, Part},
        octet::{OctetDecoder, OctetEncoder},
        text::{TextDecoder, TextEncoder},
    },
};

/// Convenience: decode a freshly-encoded `Body` back into bytes for
/// inspection.
fn body_bytes(body: Body) -> ::bytes::Bytes {
    use ::http_body_util::BodyExt;
    futures_executor::block_on(body.collect())
        .expect("collect")
        .to_bytes()
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

#[test]
fn text_encoder_writes_utf8_bytes() {
    let enc = TextEncoder::default();
    let body = enc.encode("hello").expect("encode");
    assert_eq!(body_bytes(body).as_ref(), b"hello");
    assert_eq!(
        enc.content_type().to_str().unwrap(),
        "text/plain; charset=utf-8"
    );
}

#[test]
fn text_encoder_accepts_owned_string_ref() {
    let enc = TextEncoder::default();
    let s = String::from("hello");
    let body = enc.encode(&s).expect("encode");
    assert_eq!(body_bytes(body).as_ref(), b"hello");
}

#[test]
fn text_encoder_content_type_is_overridable() {
    let enc = TextEncoder::with_content_type(::http::HeaderValue::from_static(
        "text/markdown; charset=utf-8",
    ));
    assert_eq!(
        enc.content_type().to_str().unwrap(),
        "text/markdown; charset=utf-8"
    );
}

#[test]
fn text_decoder_round_trips_utf8() {
    let dec = TextDecoder;
    let body = Body::new(Full::new(Bytes::from_static(b"\xe4\xbd\xa0\xe5\xa5\xbd"))); // 你好
    let out = futures_executor::block_on(dec.decode(body)).expect("decode");
    assert_eq!(out, "你好");
}

#[test]
fn text_decoder_rejects_invalid_utf8() {
    let dec = TextDecoder;
    let body = Body::new(Full::new(Bytes::from_static(&[0xff, 0xfe, 0xfd])));
    let err = futures_executor::block_on(dec.decode(body)).expect_err("invalid utf8");
    assert!(matches!(
        err,
        ::toac::body::codec::text::TextDecodeError::Utf8(_)
    ));
}

// ---------------------------------------------------------------------------
// Octet
// ---------------------------------------------------------------------------

#[test]
fn octet_encoder_passes_bytes_through() {
    let enc = OctetEncoder::default();
    let payload = Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]);
    let body = enc.encode(payload.clone()).expect("encode");
    assert_eq!(body_bytes(body), payload);
    assert_eq!(
        enc.content_type().to_str().unwrap(),
        "application/octet-stream"
    );
}

#[test]
fn octet_encoder_accepts_vec_input() {
    let enc = OctetEncoder::default();
    let body = enc.encode(vec![0x01, 0x02, 0x03]).expect("encode");
    assert_eq!(body_bytes(body).as_ref(), &[0x01, 0x02, 0x03]);
}

#[test]
fn octet_encoder_content_type_overridable() {
    let enc = OctetEncoder::with_content_type(::http::HeaderValue::from_static("image/png"));
    assert_eq!(enc.content_type().to_str().unwrap(), "image/png");
}

#[test]
fn octet_decoder_collects_streaming_body() {
    let dec = OctetDecoder;
    let body = Body::new(Full::new(Bytes::from_static(&[0xca, 0xfe])));
    let out = futures_executor::block_on(dec.decode(body)).expect("decode");
    assert_eq!(out.as_ref(), &[0xca, 0xfe]);
}

// ---------------------------------------------------------------------------
// Form
// ---------------------------------------------------------------------------

#[test]
fn form_encoder_serialises_struct_to_urlencoded() {
    #[derive(::serde::Serialize)]
    struct TokenRequest<'a> {
        grant_type: &'a str,
        scope: &'a str,
    }
    let enc = FormEncoder::default();
    let payload = TokenRequest {
        grant_type: "client_credentials",
        scope: "read write",
    };
    let body = enc.encode(&payload).expect("encode");
    let raw = body_bytes(body);
    let s = std::str::from_utf8(&raw).unwrap();
    // Order is preserved by serde_urlencoded; spaces become `+`.
    assert!(s.contains("grant_type=client_credentials"));
    assert!(s.contains("scope=read+write"));
    assert_eq!(
        enc.content_type().to_str().unwrap(),
        "application/x-www-form-urlencoded"
    );
}

#[test]
fn form_encoder_accepts_pair_slice() {
    let enc = FormEncoder::default();
    let pairs: [(&str, &str); 2] = [("a", "1"), ("b", "two words")];
    let body = enc.encode(&pairs).expect("encode");
    let raw = body_bytes(body);
    assert_eq!(std::str::from_utf8(&raw).unwrap(), "a=1&b=two+words");
}

// ---------------------------------------------------------------------------
// Multipart
// ---------------------------------------------------------------------------

#[test]
fn multipart_encoder_emits_canonical_boundary_layout() {
    // Pin the boundary so the assertion is deterministic.
    let enc = MultipartEncoder::with_boundary("BOUNDARY-TEST");
    let form = MultipartForm::builder()
        .text("name", "Milo")
        .file(
            "avatar",
            "milo.png",
            ::http::HeaderValue::from_static("image/png"),
            Bytes::from_static(&[0x89, 0x50, 0x4e, 0x47]),
        )
        .build();
    let body = enc.encode(&form).expect("encode");
    let raw = body_bytes(body);
    // The PNG part contains non-UTF-8 bytes, so search within the raw
    // byte slice rather than converting the whole thing to a string.
    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }
    assert!(contains(&raw, b"--BOUNDARY-TEST\r\n"));
    assert!(contains(
        &raw,
        b"Content-Disposition: form-data; name=\"name\"\r\n",
    ));
    assert!(contains(&raw, b"Milo"));
    assert!(contains(
        &raw,
        b"Content-Disposition: form-data; name=\"avatar\"; filename=\"milo.png\"\r\n",
    ));
    assert!(contains(&raw, b"Content-Type: image/png\r\n"));
    assert!(raw.ends_with(b"--BOUNDARY-TEST--\r\n"));

    // Content-Type carries the boundary parameter.
    assert_eq!(
        enc.content_type().to_str().unwrap(),
        "multipart/form-data; boundary=BOUNDARY-TEST"
    );
}

#[test]
fn multipart_quoted_values_escape_double_quotes_and_newlines() {
    let enc = MultipartEncoder::with_boundary("B");
    let form = MultipartForm::from_parts(vec![Part::text("weird\"name\r\n", "value")]);
    let body = enc.encode(&form).expect("encode");
    let s = std::str::from_utf8(body_bytes(body).as_ref())
        .unwrap()
        .to_owned();
    assert!(s.contains("name=\"weird%22name%0D%0A\""), "raw: {s}");
}

#[test]
fn multipart_default_boundary_changes_per_encoder() {
    let a = MultipartEncoder::new();
    let b = MultipartEncoder::new();
    assert_ne!(a.boundary(), b.boundary());
}

// ---------------------------------------------------------------------------
// NDJSON
// ---------------------------------------------------------------------------

#[cfg(feature = "ndjson")]
mod ndjson_tests {
    use ::bytes::Bytes;
    use ::futures_util::StreamExt;
    use ::http_body_util::Full;
    use ::serde::Deserialize;
    use ::toac::body::{
        Body,
        codec::{
            BodyDecoder,
            ndjson::{NdjsonDecodeError, NdjsonDecoder, NdjsonStream},
        },
    };

    #[derive(Debug, PartialEq, Deserialize)]
    struct Event {
        id: u32,
        name: String,
    }

    fn collect<O>(stream: NdjsonStream<O>) -> Vec<Result<O, NdjsonDecodeError>>
    where
        O: ::serde::de::DeserializeOwned + Send + 'static,
    {
        futures_executor::block_on(stream.collect())
    }

    #[test]
    fn ndjson_yields_one_event_per_line() {
        let body = Body::new(Full::new(Bytes::from_static(
            b"{\"id\":1,\"name\":\"a\"}\n{\"id\":2,\"name\":\"b\"}\n",
        )));
        let stream: NdjsonStream<Event> =
            futures_executor::block_on(NdjsonDecoder.decode(body)).expect("decode");
        let events: Vec<Event> = collect(stream).into_iter().map(|r| r.unwrap()).collect();
        assert_eq!(
            events,
            vec![
                Event {
                    id: 1,
                    name: "a".into()
                },
                Event {
                    id: 2,
                    name: "b".into()
                }
            ]
        );
    }

    #[test]
    fn ndjson_handles_trailing_line_without_newline() {
        let body = Body::new(Full::new(Bytes::from_static(
            b"{\"id\":1,\"name\":\"a\"}\n{\"id\":2,\"name\":\"b\"}",
        )));
        let stream: NdjsonStream<Event> =
            futures_executor::block_on(NdjsonDecoder.decode(body)).expect("decode");
        let events: Vec<Event> = collect(stream).into_iter().map(|r| r.unwrap()).collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].id, 2);
    }

    #[test]
    fn ndjson_skips_blank_lines_and_handles_crlf() {
        let body = Body::new(Full::new(Bytes::from_static(
            b"{\"id\":1,\"name\":\"a\"}\r\n\r\n{\"id\":2,\"name\":\"b\"}\r\n",
        )));
        let stream: NdjsonStream<Event> =
            futures_executor::block_on(NdjsonDecoder.decode(body)).expect("decode");
        let events: Vec<Event> = collect(stream).into_iter().map(|r| r.unwrap()).collect();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn ndjson_propagates_per_line_decode_error() {
        let body = Body::new(Full::new(Bytes::from_static(
            b"{\"id\":1,\"name\":\"a\"}\nnot-json\n{\"id\":3,\"name\":\"c\"}\n",
        )));
        let stream: NdjsonStream<Event> =
            futures_executor::block_on(NdjsonDecoder.decode(body)).expect("decode");
        let mut results = collect(stream).into_iter();
        assert_eq!(results.next().unwrap().unwrap().id, 1);
        let err = results.next().unwrap().unwrap_err();
        assert!(matches!(err, NdjsonDecodeError::Json(_)));
        assert_eq!(results.next().unwrap().unwrap().id, 3);
    }
}

// ---------------------------------------------------------------------------
// SSE
// ---------------------------------------------------------------------------

#[cfg(feature = "sse")]
mod sse_tests {
    use ::bytes::Bytes;
    use ::futures_util::StreamExt;
    use ::http_body_util::Full;
    use ::toac::body::{
        Body,
        codec::{
            BodyDecoder,
            sse::{Sse, SseDecoder, SseEventStream},
        },
    };

    fn collect(stream: SseEventStream) -> Vec<Sse> {
        futures_executor::block_on(stream.collect::<Vec<_>>())
            .into_iter()
            .map(|res| res.expect("sse parse"))
            .collect()
    }

    #[test]
    fn sse_decodes_named_events_and_data() {
        let body = Body::new(Full::new(Bytes::from_static(
            b"event: tick\ndata: hello\n\nevent: tick\ndata: world\n\n",
        )));
        let stream: SseEventStream =
            futures_executor::block_on(SseDecoder.decode(body)).expect("decode");
        let events = collect(stream);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event.as_deref(), Some("tick"));
        assert_eq!(events[0].data.as_deref(), Some("hello"));
        assert_eq!(events[1].data.as_deref(), Some("world"));
    }

    #[test]
    fn sse_default_event_when_unnamed() {
        let body = Body::new(Full::new(Bytes::from_static(b"data: payload\n\n")));
        let stream: SseEventStream =
            futures_executor::block_on(SseDecoder.decode(body)).expect("decode");
        let events = collect(stream);
        assert_eq!(events.len(), 1);
        assert!(events[0].event.is_none());
        assert_eq!(events[0].data.as_deref(), Some("payload"));
    }
}

// ---------------------------------------------------------------------------
// XML
// ---------------------------------------------------------------------------

#[cfg(feature = "xml")]
mod xml_tests {
    use super::body_bytes;
    use ::bytes::Bytes;
    use ::http_body_util::Full;
    use ::serde::{Deserialize, Serialize};
    use ::toac::body::{
        Body,
        codec::{
            BodyContentType, BodyDecoder, BodyEncoder,
            xml::{XmlDecodeError, XmlDecoder, XmlEncoder},
        },
    };

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename = "Pet")]
    struct Pet {
        name: String,
        age: u32,
    }

    #[test]
    fn xml_encoder_serialises_with_quick_xml() {
        let enc = XmlEncoder::default();
        let pet = Pet {
            name: "Milo".into(),
            age: 3,
        };
        let body = enc.encode(&pet).expect("encode");
        let bytes = body_bytes(body);
        let rendered = std::str::from_utf8(bytes.as_ref()).expect("utf-8");
        assert!(rendered.contains("<name>Milo</name>"), "{rendered}");
        assert!(rendered.contains("<age>3</age>"), "{rendered}");
        assert_eq!(enc.content_type().to_str().unwrap(), "application/xml");
    }

    #[test]
    fn xml_encoder_content_type_is_overridable() {
        let enc = XmlEncoder::with_content_type(::http::HeaderValue::from_static("text/xml"));
        assert_eq!(enc.content_type().to_str().unwrap(), "text/xml");
    }

    #[test]
    fn xml_decoder_round_trips() {
        let dec = XmlDecoder;
        let body = Body::new(Full::new(Bytes::from_static(
            b"<Pet><name>Milo</name><age>3</age></Pet>",
        )));
        let pet: Pet = futures_executor::block_on(dec.decode(body)).expect("decode");
        assert_eq!(
            pet,
            Pet {
                name: "Milo".into(),
                age: 3
            }
        );
    }

    #[test]
    fn xml_decoder_reports_decode_error() {
        let dec = XmlDecoder;
        let body = Body::new(Full::new(Bytes::from_static(b"<Pet><name>Milo")));
        let err = futures_executor::block_on(<XmlDecoder as BodyDecoder<Pet>>::decode(&dec, body))
            .expect_err("incomplete xml");
        assert!(matches!(err, XmlDecodeError::Xml(_)));
    }
}
