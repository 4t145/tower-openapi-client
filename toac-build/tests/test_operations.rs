use indoc::indoc;

fn generate(spec_yaml: &str) -> String {
    let spec = oas3::from_yaml(spec_yaml).expect("spec parses");
    let tokens = toac_build::build(&spec).expect("codegen");
    let file = syn::parse_file(&tokens.to_string()).expect("valid Rust");
    prettyplease::unparse(&file)
}

#[test]
fn get_with_path_param_emits_request_and_response() {
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
                - name: limit
                  in: query
                  schema: { type: integer }
              responses:
                "200":
                  description: OK
                  content:
                    application/json:
                      schema:
                        $ref: "#/components/schemas/Pet"
                "404":
                  description: missing
    "##});

    // Types live under operations::pets::by_id::get::*.
    assert!(
        rendered.contains("pub mod pets"),
        "missing `pets` mod:\n{rendered}"
    );
    assert!(
        rendered.contains("pub mod by_id"),
        "missing `by_id` mod:\n{rendered}"
    );
    assert!(
        rendered.contains("pub mod get"),
        "missing `get` mod:\n{rendered}"
    );
    assert!(
        rendered.contains("pub struct Request"),
        "request struct name should collapse to `Request`:\n{rendered}"
    );
    assert!(
        rendered.contains("pub id: String"),
        "path param missing:\n{rendered}"
    );
    assert!(
        rendered.contains("pub limit: Option<i64>"),
        "query param missing or wrong optionality:\n{rendered}"
    );
    assert!(
        rendered.contains("pub enum Response"),
        "response enum name should collapse to `Response`:\n{rendered}"
    );
    assert!(
        rendered.contains("Status200"),
        "missing 200 variant:\n{rendered}"
    );
    assert!(
        rendered.contains("Status404"),
        "missing 404 variant:\n{rendered}"
    );
    assert!(rendered.contains("pub const METHOD: ::http::Method = ::http::Method::GET"));
    assert!(rendered.contains(r#"pub const PATH_TEMPLATE: &'static str = "/pets/{id}""#));
}

#[test]
fn post_with_request_body_adds_body_field() {
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
                  content:
                    application/json:
                      schema:
                        $ref: "#/components/schemas/NewPet"
                default:
                  description: any other status
    "##});

    // POST /pets lives under operations::pets::post::*.
    assert!(rendered.contains("pub mod pets"));
    assert!(
        rendered.contains("pub mod post"),
        "missing `post` method mod:\n{rendered}"
    );
    assert!(rendered.contains("pub struct Request"));
    assert!(
        rendered.contains("pub body: crate::components::NewPet"),
        "body field should reference crate::components::NewPet:\n{rendered}"
    );
    assert!(rendered.contains("pub enum Response"));
    assert!(rendered.contains("Status201"));
    assert!(
        rendered.contains("Default"),
        "default branch missing:\n{rendered}"
    );
}

#[test]
fn op_without_operation_id_still_lands_in_correct_mod() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /pets/{id}/favourite:
            put:
              responses:
                "204":
                  description: no content
    "##});

    // Module tree follows the URL template + HTTP method, regardless
    // of whether the op declares an `operationId`.
    assert!(
        rendered.contains("pub mod pets"),
        "pets mod missing:\n{rendered}"
    );
    assert!(
        rendered.contains("pub mod by_id"),
        "by_id mod missing:\n{rendered}"
    );
    assert!(
        rendered.contains("pub mod favourite"),
        "favourite mod missing:\n{rendered}"
    );
    assert!(
        rendered.contains("pub mod put"),
        "put method mod missing:\n{rendered}"
    );
    assert!(rendered.contains("pub struct Request"));
}

#[test]
fn ignored_headers_do_not_become_fields() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /things:
            get:
              operationId: listThings
              parameters:
                - name: Accept
                  in: header
                  schema: { type: string }
                - name: Content-Type
                  in: header
                  schema: { type: string }
                - name: Authorization
                  in: header
                  schema: { type: string }
                - name: X-Trace-Id
                  in: header
                  schema: { type: string }
              responses:
                "200":
                  description: ok
    "##});

    // GET /things → operations::things::get::Request
    assert!(rendered.contains("pub mod things"));
    assert!(rendered.contains("pub mod get"));
    assert!(rendered.contains("pub struct Request"));
    assert!(
        rendered.contains("pub x_trace_id"),
        "trace header should remain:\n{rendered}"
    );
    assert!(!rendered.contains("pub accept"));
    assert!(!rendered.contains("pub content_type"));
    assert!(!rendered.contains("pub authorization"));
}

#[test]
fn colliding_param_names_get_location_suffix() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /items/{id}:
            get:
              operationId: getItem
              parameters:
                - name: id
                  in: path
                  required: true
                  schema: { type: string }
                - name: id
                  in: query
                  schema: { type: string }
              responses:
                "200":
                  description: ok
    "##});

    assert!(rendered.contains("pub id: String"));
    assert!(
        rendered.contains("pub id_query: Option<String>"),
        "second id should be suffixed:\n{rendered}"
    );
    // The wire name is applied inside MakeRequest rather than via a
    // serde rename — the query rendering pushes the literal "id" key.
    assert!(
        rendered.contains("__path.push_str(\"id\")"),
        "wire name not carried into query encoding:\n{rendered}"
    );
}

#[test]
fn path_and_operation_parameter_merge_operation_wins() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /x/{id}:
            parameters:
              - name: id
                in: path
                required: true
                schema: { type: integer, format: int32 }
            get:
              operationId: getX
              parameters:
                - name: id
                  in: path
                  required: true
                  description: overridden
                  schema: { type: string }
              responses:
                "200":
                  description: ok
    "##});

    // Operation-level override wins: id should be String, not i32.
    assert!(
        rendered.contains("pub id: String"),
        "operation-level override should win:\n{rendered}"
    );
    assert!(
        !rendered.contains("pub id: i32"),
        "path-level schema should have been replaced:\n{rendered}"
    );
}

/// When an op's response variant payload references a component schema
/// whose name collides with the op-private `Response` enum (e.g. OpenAI's
/// `POST /responses` returning `#/components/schemas/Response`), the
/// payload reference must be qualified to `crate::components::Response`.
/// Without that, the bare `Response` ident inside the variant resolves
/// to the enum itself and produces an infinite-size recursive type.
#[test]
fn op_response_payload_qualified_when_component_shares_name() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          schemas:
            Response:
              type: object
              required: [id]
              properties:
                id: { type: string }
        paths:
          /responses:
            post:
              operationId: createResponse
              responses:
                "200":
                  description: OK
                  content:
                    application/json:
                      schema:
                        $ref: "#/components/schemas/Response"
    "##});

    assert!(
        rendered.contains("Status200(crate::components::Response)"),
        "variant payload must be absolute-qualified to break the \
         self-shadowing cycle:\n{rendered}"
    );
    assert!(
        !rendered.contains("Status200(Response)"),
        "bare `Response` would resolve to the enum itself:\n{rendered}"
    );
}
