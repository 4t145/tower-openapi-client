//! Shape tests for the generated `security` module and its
//! per-operation integration.

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
fn bearer_scheme_emits_auth_config_and_credential() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          securitySchemes:
            bearer:
              type: http
              scheme: bearer
        security:
          - bearer: []
        paths:
          /ping:
            get:
              operationId: ping
              responses:
                "200":
                  description: ok
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("pub mod security"),
        "security module missing:\n{rendered}"
    );
    assert!(
        compact.contains("pub struct BearerCredential"),
        "bearer credential wrapper missing:\n{rendered}"
    );
    assert!(
        compact.contains("pub struct AuthConfig"),
        "AuthConfig missing:\n{rendered}"
    );
    assert!(
        compact.contains("pub struct AuthConfigBuilder"),
        "builder missing:\n{rendered}"
    );
    assert!(
        compact.contains("pub fn builder() -> AuthConfigBuilder"),
        "builder entry point missing:\n{rendered}"
    );
    assert!(
        compact.contains("impl ::toac::AuthSelector for AuthConfig"),
        "AuthSelector impl missing:\n{rendered}"
    );
    // Op inherits spec-level security → SECURITY const + Extensions insert.
    assert!(
        compact
            .contains("pub const SECURITY: &'static [&'static [&'static str]] = &[&[\"bearer\"]]"),
        "SECURITY const should list bearer alternative:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::OperationSecurity(&[&[\"bearer\"]])"),
        "make_request should attach OperationSecurity extension:\n{rendered}"
    );
}

#[test]
fn api_key_scheme_wires_through_to_runtime() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          securitySchemes:
            my_api_key:
              type: apiKey
              name: X-API-Key
              in: header
        paths:
          /ping:
            get:
              operationId: ping
              security:
                - my_api_key: []
              responses:
                "200":
                  description: ok
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("pub struct MyApiKeyCredential"),
        "API key wrapper missing:\n{rendered}"
    );
    // Wrapper should project into the runtime credential with the
    // header name + location baked in.
    assert!(
        compact.contains("::toac::security::ApiKeyCredential"),
        "runtime ApiKeyCredential reference missing:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::security::ApiKeyLocation::Header"),
        "header location missing:\n{rendered}"
    );
    assert!(
        compact.contains("name: \"X-API-Key\""),
        "wire name should be preserved:\n{rendered}"
    );
    // SECURITY const uses the spec key, not the wire name.
    assert!(
        compact.contains("&[&[\"my_api_key\"]]"),
        "SECURITY should reference spec name:\n{rendered}"
    );
}

#[test]
fn basic_scheme_exposes_username_password_setter() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          securitySchemes:
            basic:
              type: http
              scheme: basic
        paths:
          /ping:
            get:
              operationId: ping
              security:
                - basic: []
              responses:
                "200":
                  description: ok
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("pub struct BasicCredential"),
        "Basic wrapper missing:\n{rendered}"
    );
    // Basic takes two arguments — username and password.
    assert!(
        compact.contains("pub fn basic<U, P>(mut self, username: U, password: P)"),
        "basic setter signature missing:\n{rendered}"
    );
}

#[test]
fn unsupported_scheme_in_components_is_silently_skipped() {
    // OAuth2 / OpenID Connect / mutualTLS aren't supported yet, but
    // specs routinely declare them alongside bearer/apiKey. Codegen
    // should proceed, emitting only the supported wrappers.
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          securitySchemes:
            bearer:
              type: http
              scheme: bearer
            oauth2_flow:
              type: oauth2
              flows:
                clientCredentials:
                  tokenUrl: https://example/token
                  scopes: {}
            mtls:
              type: mutualTLS
        paths:
          /ping:
            get:
              operationId: ping
              security:
                - bearer: []
              responses:
                "200":
                  description: ok
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("pub struct BearerCredential"),
        "bearer still emitted:\n{rendered}"
    );
    // Unsupported schemes get no wrapper type.
    assert!(
        !compact.contains("pub struct Oauth2FlowCredential"),
        "oauth2 should not materialise a wrapper:\n{rendered}"
    );
    assert!(
        !compact.contains("pub struct MtlsCredential"),
        "mutualTLS should not materialise a wrapper:\n{rendered}"
    );
}

#[test]
fn unsupported_alternative_in_op_security_is_dropped() {
    // Op declares OR between oauth2 (unsupported) and api_key (supported).
    // The oauth2 alternative is dropped so the generated SECURITY const
    // names only api_key; the op itself still compiles.
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          securitySchemes:
            api_key:
              type: apiKey
              name: X-API-Key
              in: header
            oauth2_flow:
              type: oauth2
              flows:
                clientCredentials:
                  tokenUrl: https://example/token
                  scopes: {}
        paths:
          /pets:
            get:
              operationId: listPets
              security:
                - oauth2_flow: []
                - api_key: []
              responses:
                "200":
                  description: ok
    "##});

    let compact = compact(&rendered);
    assert!(
        compact.contains("&[&[\"api_key\"]]"),
        "only the supported alternative should remain:\n{rendered}"
    );
    assert!(
        !compact.contains("\"oauth2_flow\""),
        "oauth2 alternative should be dropped:\n{rendered}"
    );
}

#[test]
fn op_without_security_has_empty_security_const() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        paths:
          /ping:
            get:
              operationId: ping
              responses:
                "200":
                  description: ok
    "##});

    let compact = compact(&rendered);
    // Public endpoint: const present but empty outer slice.
    assert!(
        compact.contains("pub const SECURITY: &'static [&'static [&'static str]] = &[]"),
        "public op should expose empty SECURITY const:\n{rendered}"
    );
    // No OperationSecurity extension insert.
    assert!(
        !compact.contains("::toac::OperationSecurity"),
        "public op should not insert the extension:\n{rendered}"
    );
}

#[test]
fn op_inherits_spec_level_security_when_op_level_absent() {
    // Spec-level declares bearer; ops without their own `security`
    // inherit it. (Distinguishing "unset" from explicit `security: []`
    // isn't possible through the `oas3` 0.21 model — both collapse to
    // an empty Vec. See TODO.md's Security section.)
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        components:
          securitySchemes:
            bearer:
              type: http
              scheme: bearer
        security:
          - bearer: []
        paths:
          /resource:
            get:
              operationId: getResource
              responses:
                "200":
                  description: ok
    "##});

    let compact = compact(&rendered);
    // Op inherits the spec-level requirement verbatim.
    assert!(
        compact
            .contains("pub const SECURITY: &'static [&'static [&'static str]] = &[&[\"bearer\"]]"),
        "op should inherit spec-level bearer requirement:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::OperationSecurity(&[&[\"bearer\"]])"),
        "extension should carry the inherited requirement:\n{rendered}"
    );
}
