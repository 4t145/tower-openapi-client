//! Checks that the generated runtime trait impls have the shape we
//! expect. Execution-level behaviour is covered separately (end-to-end
//! tests with a mock server are out of this crate's scope for now).

use indoc::indoc;

fn generate(spec_yaml: &str) -> String {
    let spec = oas3::from_yaml(spec_yaml).expect("spec parses");
    let tokens = toac_build::build(&spec).expect("codegen");
    let file = syn::parse_file(&tokens.to_string()).expect("valid Rust");
    prettyplease::unparse(&file)
}

/// Whitespace-collapsed view of `rendered` so substring assertions aren't
/// tripped up by prettyplease's line wrapping. Also strips trailing
/// commas that prettyplease inserts on wrapped generic argument lists.
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
fn make_request_emitted_for_get_with_path_and_query() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /pets/{id}:
            get:
              operationId: getPet
              parameters:
                - name: id
                  in: path
                  required: true
                  schema: { type: string }
                - name: limit
                  in: query
                  schema: { type: integer }
                - name: X-Trace
                  in: header
                  schema: { type: string }
              responses:
                "200":
                  description: ok
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("impl ::toac::MakeRequest for Request"),
        "MakeRequest impl (non-generic) not found:\ncompact:\n{compact}\nrendered:\n{rendered}"
    );
    assert!(
        compact.contains("pub mod pets")
            && compact.contains("pub mod by_id")
            && compact.contains("pub mod get"),
        "op not emitted under operations::pets::by_id::get:\n{rendered}"
    );
    assert!(
        compact.contains("Output = ::std::result::Result<::toac::Request, Self::Error>"),
        "make_request future output not Result<::toac::Request, Self::Error>:\n{rendered}"
    );
    // Op declares a query parameter (`limit`), so the Error widens
    // to `EncodeRequestError` to carry encoder failures.
    assert!(
        compact.contains("type Error = ::toac::EncodeRequestError"),
        "operation with query params should use EncodeRequestError:\n{rendered}"
    );
    assert!(
        compact.contains("fn make_request(self)"),
        "impl method missing:\n{rendered}"
    );
    // path template substitution
    assert!(
        compact.contains("__path.push_str(\"/pets/\")"),
        "path literal segment not rendered:\n{rendered}"
    );
    assert!(
        compact.contains("&self.id"),
        "path placeholder not bound to self.id:\n{rendered}"
    );
    // query param appended via helper
    assert!(
        compact.contains("__query_first"),
        "query state var missing:\n{rendered}"
    );
    assert!(
        compact.contains("self.limit"),
        "query field not referenced:\n{rendered}"
    );
    // header injection
    assert!(
        compact.contains(".header(\"X-Trace\""),
        "header param not injected:\n{rendered}"
    );
    // empty body uses Body::empty()
    assert!(
        compact.contains("::toac::body::Body::empty()"),
        "empty body not rendered via toac::body::Body::empty():\n{rendered}"
    );
}

#[test]
fn make_request_wraps_body_for_post() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            NewPet:
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
                  application/json:
                    schema:
                      $ref: "#/components/schemas/NewPet"
              responses:
                "201":
                  description: created
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("::toac::body::codec::encode_body"),
        "body not encoded via codec::encode_body:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::body::codec::json::JsonEncoder"),
        "JSON encoder not selected:\n{rendered}"
    );
    // Plain `application/json` — default encoder, no Content-Type override.
    assert!(
        !compact.contains("HeaderValue::from_static"),
        "plain application/json should not emit a Content-Type override:\n{rendered}"
    );
    assert!(
        compact.contains("&self.body"),
        "body field not referenced:\n{rendered}"
    );
    assert!(
        compact.contains("type Error = ::serde_json::Error"),
        "JSON-body operation should propagate serde_json::Error:\n{rendered}"
    );
}

#[test]
fn vendor_json_mime_overrides_content_type() {
    // `application/vnd.github+json` is a common "JSON with a vendor
    // sub-type" MIME. The generator should reuse JsonEncoder's serde
    // path but override the Content-Type header so the wire value
    // matches the spec exactly.
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            NewPet:
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
                  application/vnd.github+json:
                    schema:
                      $ref: "#/components/schemas/NewPet"
              responses:
                "201":
                  description: created
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("::toac::body::codec::json::JsonEncoder"),
        "JSON encoder should still be picked for +json:\n{rendered}"
    );
    assert!(
        compact.contains("::http::HeaderValue::from_static(\"application/vnd.github+json\")"),
        "Content-Type override missing or wrong value:\n{rendered}"
    );
    assert!(
        compact.contains("content_type: ::http::HeaderValue::from_static"),
        "encoder struct literal should set the content_type field:\n{rendered}"
    );
}

#[test]
fn parse_response_dispatches_on_status() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            Pet:
              type: object
              required: [id]
              properties:
                id: { type: string }
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
                  description: OK
                  content:
                    application/json:
                      schema:
                        $ref: "#/components/schemas/Pet"
                "404":
                  description: missing
                default:
                  description: fallback
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("impl ::toac::ParseResponse for Response"),
        "ParseResponse impl not found:\ncompact:\n{compact}\nrendered:\n{rendered}"
    );
    assert!(
        compact.contains("response: ::http::Response<__B>"),
        "parse_response does not take a generic ::http::Response<B>:\n{rendered}"
    );
    assert!(
        compact.contains("__B: ::http_body::Body<Data = ::bytes::Bytes>"),
        "parse_response body bound missing:\n{rendered}"
    );
    assert!(
        rendered.contains("type Error = ::toac::DecodeError"),
        "associated Error type wrong:\n{rendered}"
    );
    // known numeric status arms
    assert!(
        rendered.contains("200u16 =>") || rendered.contains("200 =>"),
        "200 arm missing:\n{rendered}"
    );
    assert!(
        rendered.contains("404u16 =>") || rendered.contains("404 =>"),
        "404 arm missing:\n{rendered}"
    );
    // default is the fallback variant on the body enum
    assert!(
        rendered.contains("ResponseBody::Default"),
        "default fallback missing:\n{rendered}"
    );
    // 200 with schema decodes via the codec
    assert!(
        compact.contains("::toac::body::codec::decode_body"),
        "body not decoded via codec::decode_body:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::body::codec::json::JsonDecoder"),
        "JSON decoder not selected:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::DecodeError::Codec"),
        "codec error not wrapped via DecodeError::Codec:\n{rendered}"
    );
}

#[test]
fn component_refs_from_operations_are_qualified() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            Pet:
              type: object
              required: [id]
              properties:
                id: { type: string }
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
                  description: OK
                  content:
                    application/json:
                      schema:
                        $ref: "#/components/schemas/Pet"
    "##});

    // The `Pet` variant payload inside the operations module must point
    // back at the components module through the absolute `crate::`
    // path — operations are nested several levels deep under
    // `operations::<path>::<method>`, so a relative `super::` path
    // would have to vary per op.
    assert!(
        rendered.contains("crate::components::Pet"),
        "component reference should use crate::components::...:\n{rendered}"
    );
    // Local types like the op's own response body enum must NOT be rewritten.
    assert!(
        rendered.contains("pub enum ResponseBody"),
        "response body enum ident mangled:\n{rendered}"
    );
}

#[test]
fn operation_impl_emitted_for_each_operation() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            NewPet:
              type: object
              required: [name]
              properties:
                name: { type: string }
        paths:
          /ping:
            get:
              operationId: ping
              responses:
                "204":
                  description: no content
          /pets:
            post:
              operationId: createPet
              requestBody:
                required: true
                content:
                  application/json:
                    schema:
                      $ref: "#/components/schemas/NewPet"
              responses:
                "201":
                  description: created
    "##});

    let compact = compact(&rendered);

    // Both operations share the same Operation trait with no body-type
    // associated type; they only bind the response enum. Each op's
    // `Request` / `Response` types live in their own path-derived mod,
    // so the `impl` block references them by the unqualified local
    // names inside that mod.
    assert!(
        compact.contains("pub mod ping"),
        "ping method mod missing:\n{rendered}"
    );
    assert!(
        compact.contains("pub mod pets") && compact.contains("pub mod post"),
        "createPet mod structure missing:\n{rendered}"
    );
    // The impl block is emitted inside each op's mod with local names.
    let operation_impl_count = compact
        .matches("impl ::toac::Operation for Request")
        .count();
    assert_eq!(
        operation_impl_count, 2,
        "expected one Operation impl per op, got {operation_impl_count}:\n{rendered}"
    );
    assert!(
        compact.matches("type Response = Response").count() >= 2,
        "each op should bind `type Response = Response`:\n{rendered}"
    );

    // RequestBody associated type is gone.
    assert!(
        !compact.contains("type RequestBody"),
        "stale RequestBody associated type still emitted:\n{rendered}"
    );
}

#[test]
fn object_query_param_uses_runtime_encoder() {
    // Regression for B2: an object-typed query parameter (here a
    // free-form map) must not stringify through `ToString`. The
    // generator should hand the value to the runtime encoder
    // alongside the spec-supplied `(style, explode)` pair.
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /search:
            get:
              operationId: search
              parameters:
                - name: filter
                  in: query
                  style: deepObject
                  explode: true
                  schema:
                    type: object
                    additionalProperties: { type: string }
              responses:
                "200":
                  description: ok
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("encode_serialized"),
        "object query param should route through encode_serialized:\n{rendered}"
    );
    assert!(
        compact.contains("ParameterStyle::DeepObject"),
        "deepObject style should be preserved on the wire:\n{rendered}"
    );
    assert!(
        !compact.contains("ToString::to_string(&__value)"),
        "blanket ToString path must not be used for object query params:\n{rendered}"
    );
    assert!(
        compact.contains("type Error = ::toac::EncodeRequestError"),
        "op should widen Error to EncodeRequestError:\n{rendered}"
    );
}

#[test]
fn unknown_status_without_default_returns_error() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /ping:
            get:
              operationId: ping
              responses:
                "204":
                  description: ok
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("::toac::DecodeError::UnexpectedStatus(__status)"),
        "UnexpectedStatus fallback missing:\ncompact:\n{compact}\nrendered:\n{rendered}"
    );
}
