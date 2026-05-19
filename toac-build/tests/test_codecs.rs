//! Codegen shape tests covering the non-JSON codecs. Asserts the
//! generator picks the right encoder / decoder type and that the
//! request body field collapses to the codec's expected payload type.

use indoc::indoc;

fn generate(spec_yaml: &str) -> String {
    let spec = oas3::from_yaml(spec_yaml).expect("spec parses");
    let tokens = toac_build::build(&spec).expect("codegen");
    let file = syn::parse_file(&tokens.to_string()).expect("valid Rust");
    prettyplease::unparse(&file)
}

fn compact(rendered: &str) -> String {
    let joined = rendered.split_whitespace().collect::<Vec<_>>().join(" ");
    joined
        .replace(", >", ">")
        .replace(",>", ">")
        .replace("< ", "<")
        .replace(" >", ">")
        .replace(", )", ")")
        .replace("( ", "(")
        .replace(" )", ")")
}

#[test]
fn form_urlencoded_request_body_uses_form_encoder() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            TokenRequest:
              type: object
              required: [grant_type]
              properties:
                grant_type: { type: string }
                scope: { type: string }
        paths:
          /oauth/token:
            post:
              operationId: token
              requestBody:
                required: true
                content:
                  application/x-www-form-urlencoded:
                    schema:
                      $ref: "#/components/schemas/TokenRequest"
              responses:
                "200":
                  description: ok
    "##});

    let compact = compact(&rendered);
    // Body field still typed by the schema (form encodes serde shapes).
    assert!(
        compact.contains("pub body: crate::components::TokenRequest"),
        "form body should follow the schema:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::body::codec::form::FormEncoder"),
        "form encoder not selected:\n{rendered}"
    );
    assert!(
        compact.contains("type Error = ::serde_urlencoded::ser::Error"),
        "form-body op should propagate serde_urlencoded's error:\n{rendered}"
    );
}

#[test]
fn octet_stream_request_body_uses_bytes_payload() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /upload:
            post:
              operationId: uploadBlob
              requestBody:
                required: true
                content:
                  application/octet-stream:
                    schema: { type: string, format: binary }
              responses:
                "204":
                  description: ok
    "##});

    let compact = compact(&rendered);
    // Schema is ignored — payload is always bytes::Bytes.
    assert!(
        compact.contains("pub body: ::bytes::Bytes"),
        "octet-stream body should be bytes::Bytes:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::body::codec::octet::OctetEncoder"),
        "octet encoder not selected:\n{rendered}"
    );
    // Encoder default works because spec MIME matches default exactly.
    assert!(
        !compact.contains("HeaderValue::from_static"),
        "no Content-Type override expected for plain octet-stream:\n{rendered}"
    );
    // Encoder is infallible.
    assert!(
        compact.contains("type Error = ::std::convert::Infallible"),
        "octet-stream op should be infallible:\n{rendered}"
    );
}

#[test]
fn image_mime_picks_octet_codec_with_override() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /upload:
            post:
              operationId: uploadImage
              requestBody:
                required: true
                content:
                  image/png:
                    schema: { type: string, format: binary }
              responses:
                "204":
                  description: ok
    "##});

    let compact = compact(&rendered);
    assert!(compact.contains("pub body: ::bytes::Bytes"));
    // image/png → octet codec but Content-Type overridden.
    assert!(
        compact.contains("::http::HeaderValue::from_static(\"image/png\")"),
        "image/png override missing:\n{rendered}"
    );
}

#[test]
fn text_plain_request_body_uses_string_payload() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /note:
            post:
              operationId: postNote
              requestBody:
                required: true
                content:
                  text/plain:
                    schema: { type: string }
              responses:
                "204":
                  description: ok
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("pub body: ::std::string::String"),
        "text body should be String:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::body::codec::text::TextEncoder"),
        "text encoder not selected:\n{rendered}"
    );
    // Spec MIME `text/plain` (no charset param) → encoder default
    // (`text/plain; charset=utf-8`) doesn't match, so an override is emitted.
    assert!(
        compact.contains("::http::HeaderValue::from_static(\"text/plain\")"),
        "Content-Type override expected because spec MIME differs from codec default:\n{rendered}"
    );
}

#[test]
fn multipart_request_body_uses_multipart_form_payload() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /upload:
            post:
              operationId: uploadMixed
              requestBody:
                required: true
                content:
                  multipart/form-data:
                    schema:
                      type: object
                      properties:
                        avatar: { type: string, format: binary }
                        caption: { type: string }
              responses:
                "204":
                  description: ok
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("pub body: ::toac::body::codec::multipart::MultipartForm"),
        "multipart body should be MultipartForm:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::body::codec::multipart::MultipartEncoder::new()"),
        "multipart encoder not selected:\n{rendered}"
    );
}

#[test]
fn octet_stream_response_decodes_to_bytes() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /download:
            get:
              operationId: downloadBlob
              responses:
                "200":
                  description: ok
                  content:
                    application/octet-stream:
                      schema: { type: string, format: binary }
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("Status200(::bytes::Bytes)"),
        "octet response variant should hold Bytes:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::body::codec::octet::OctetDecoder"),
        "octet decoder not selected:\n{rendered}"
    );
}

#[test]
fn xml_request_body_uses_xml_encoder_with_schema_payload() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            Pet:
              type: object
              required: [name]
              properties:
                name: { type: string }
        paths:
          /pets:
            post:
              operationId: createPet
              requestBody:
                required: true
                content:
                  application/xml:
                    schema:
                      $ref: "#/components/schemas/Pet"
              responses:
                "201":
                  description: created
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("pub body: crate::components::Pet"),
        "xml body should follow the schema:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::body::codec::xml::XmlEncoder"),
        "xml encoder not selected:\n{rendered}"
    );
    assert!(
        compact.contains("type Error = ::toac::body::codec::xml::XmlEncodeError"),
        "xml-body op should propagate quick_xml's serialise error:\n{rendered}"
    );
}

#[test]
fn xml_vendor_mime_overrides_content_type() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            Feed:
              type: object
              properties:
                title: { type: string }
        paths:
          /feed:
            post:
              operationId: postFeed
              requestBody:
                required: true
                content:
                  application/atom+xml:
                    schema:
                      $ref: "#/components/schemas/Feed"
              responses:
                "204":
                  description: ok
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("::toac::body::codec::xml::XmlEncoder"),
        "xml encoder not selected for +xml MIME:\n{rendered}"
    );
    assert!(
        compact.contains("::http::HeaderValue::from_static(\"application/atom+xml\")"),
        "vendor +xml MIME should override Content-Type:\n{rendered}"
    );
}

#[test]
fn xml_response_decodes_through_xml_decoder() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            Pet:
              type: object
              properties:
                name: { type: string }
        paths:
          /pets/{id}:
            get:
              operationId: getPet
              parameters:
                - name: id
                  in: path
                  required: true
                  schema: { type: string }
              responses:
                "200":
                  description: ok
                  content:
                    application/xml:
                      schema:
                        $ref: "#/components/schemas/Pet"
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("Status200(crate::components::Pet)"),
        "xml response variant should hold the schema type:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::body::codec::xml::XmlDecoder"),
        "xml decoder not selected:\n{rendered}"
    );
}

#[test]
fn ndjson_response_wraps_schema_in_stream() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            Event:
              type: object
              properties:
                id: { type: integer }
        paths:
          /events:
            get:
              operationId: streamEvents
              responses:
                "200":
                  description: ok
                  content:
                    application/x-ndjson:
                      schema:
                        $ref: "#/components/schemas/Event"
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains(
            "Status200(::toac::body::codec::ndjson::NdjsonStream<crate::components::Event>)",
        ),
        "ndjson response variant should wrap schema in NdjsonStream:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::body::codec::ndjson::NdjsonDecoder"),
        "ndjson decoder not selected:\n{rendered}"
    );
}

#[test]
fn ndjson_response_without_schema_falls_back_to_json_value() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /events:
            get:
              operationId: streamEventsRaw
              responses:
                "200":
                  description: ok
                  content:
                    application/jsonl: {}
    "##});

    let compact = compact(&rendered);
    assert!(
        compact
            .contains("Status200(::toac::body::codec::ndjson::NdjsonStream<::serde_json::Value>)",),
        "schemaless ndjson response should fall back to serde_json::Value:\n{rendered}"
    );
}

#[test]
fn sse_response_decodes_to_event_stream() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /tail:
            get:
              operationId: tailLogs
              responses:
                "200":
                  description: ok
                  content:
                    text/event-stream:
                      schema: { type: string }
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("Status200(::toac::body::codec::sse::SseEventStream)"),
        "sse response variant should be SseEventStream:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::body::codec::sse::SseDecoder"),
        "sse decoder not selected:\n{rendered}"
    );
}

#[test]
fn text_plain_response_decodes_to_string() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /readme:
            get:
              operationId: getReadme
              responses:
                "200":
                  description: ok
                  content:
                    text/plain:
                      schema: { type: string }
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("Status200(::std::string::String)"),
        "text response variant should hold String:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::body::codec::text::TextDecoder"),
        "text decoder not selected:\n{rendered}"
    );
}
