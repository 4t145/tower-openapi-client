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
        compact.contains("impl ::toac::MakeRequest for GetPetRequest"),
        "MakeRequest impl (non-generic) not found:\ncompact:\n{compact}\nrendered:\n{rendered}"
    );
    assert!(
        compact.contains("Output = ::toac::Request"),
        "make_request future output not ::toac::Request:\n{rendered}"
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

    assert!(
        rendered.contains("::toac::body::Body::new"),
        "body not wrapped through toac::body::Body::new:\n{rendered}"
    );
    assert!(
        rendered.contains("::http_body_util::Full::new"),
        "Full<Bytes> inner body not used:\n{rendered}"
    );
    assert!(
        rendered.contains("::serde_json::to_vec(&self.body)"),
        "body field not JSON-serialised:\n{rendered}"
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
        compact.contains("impl ::toac::ParseResponse for GetPetResponse"),
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
    // default is the fallback variant
    assert!(
        rendered.contains("GetPetResponse::Default"),
        "default fallback missing:\n{rendered}"
    );
    // 200 with schema decodes JSON
    assert!(
        rendered.contains("::serde_json::from_slice(__bytes.as_ref())"),
        "JSON decode not emitted:\n{rendered}"
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
    // back at the components module or it won't compile.
    assert!(
        rendered.contains("super::components::Pet"),
        "component reference in operations module not qualified:\n{rendered}"
    );
    // Local types like the op's own response enum must NOT be rewritten.
    assert!(
        rendered.contains("pub enum GetPetResponse"),
        "response enum ident mangled:\n{rendered}"
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
    // associated type; they only bind the response enum.
    assert!(
        compact.contains("impl ::toac::Operation for PingRequest"),
        "PingRequest Operation impl missing:\ncompact:\n{compact}"
    );
    assert!(
        compact.contains("type Response = PingResponse"),
        "Ping Response type not bound:\n{rendered}"
    );

    assert!(
        compact.contains("impl ::toac::Operation for CreatePetRequest"),
        "CreatePetRequest Operation impl missing:\n{rendered}"
    );
    assert!(
        compact.contains("type Response = CreatePetResponse"),
        "CreatePet Response type not bound:\n{rendered}"
    );

    // RequestBody associated type is gone.
    assert!(
        !compact.contains("type RequestBody"),
        "stale RequestBody associated type still emitted:\n{rendered}"
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
