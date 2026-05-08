//! Shape tests for the generated `servers` module.

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
fn no_servers_falls_back_to_root_url() {
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
        compact.contains("pub struct ServerOption0"),
        "ServerOption0 not emitted:\n{rendered}"
    );
    assert!(
        compact.contains("impl ::toac::Server for ServerOption0"),
        "Server impl missing:\n{rendered}"
    );
    assert!(
        compact.contains("::std::borrow::Cow::Borrowed(\"/\")"),
        "fallback URL `/` not embedded:\n{rendered}"
    );
    assert!(
        compact.contains("pub type ApiServer = ServerOption0"),
        "single-option aggregate should alias ServerOption0:\n{rendered}"
    );
}

#[test]
fn multiple_servers_produce_aggregate_enum() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        servers:
          - url: https://api.example.com
            description: Production
          - url: https://sandbox.example.com
            description: Sandbox
        paths: {}
    "##});
    let compact = compact(&rendered);
    assert!(
        compact.contains("pub struct ServerOption0"),
        "option 0 missing:\n{rendered}"
    );
    assert!(
        compact.contains("pub struct ServerOption1"),
        "option 1 missing:\n{rendered}"
    );
    assert!(
        compact.contains("pub enum ApiServer"),
        "aggregate enum missing:\n{rendered}"
    );
    assert!(
        compact.contains("Option0(ServerOption0)"),
        "Option0 variant missing:\n{rendered}"
    );
    assert!(
        compact.contains("Option1(ServerOption1)"),
        "Option1 variant missing:\n{rendered}"
    );
    assert!(
        compact.contains("impl ::toac::Server for ApiServer"),
        "aggregate Server impl missing:\n{rendered}"
    );
    assert!(
        compact.contains("impl ::std::convert::From<ServerOption0> for ApiServer"),
        "From<ServerOption0> missing:\n{rendered}"
    );
}

#[test]
fn templated_server_generates_fields_and_enum_variable() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        servers:
          - url: https://{region}.api.example.com/v{ver}
            variables:
              region:
                default: us
                enum: [us, eu, ap]
              ver:
                default: "2"
        paths: {}
    "##});
    let compact = compact(&rendered);
    assert!(
        compact.contains("pub struct ServerOption0"),
        "option struct missing:\n{rendered}"
    );
    assert!(
        compact.contains("pub region:"),
        "region field missing:\n{rendered}"
    );
    assert!(
        compact.contains("pub ver:"),
        "ver field missing:\n{rendered}"
    );
    assert!(
        compact.contains("pub enum ServerOption0Region"),
        "nested enum for region missing:\n{rendered}"
    );
    assert!(
        compact.contains("impl ::std::fmt::Display for ServerOption0Region"),
        "Display for region enum missing:\n{rendered}"
    );
    // URL rendering goes through format!
    assert!(
        compact.contains("::std::format!"),
        "templated URL should use format!():\n{rendered}"
    );
    // Default impl seeds variables
    assert!(
        compact.contains("impl ::std::default::Default for ServerOption0"),
        "Default impl missing:\n{rendered}"
    );
}

#[test]
fn operation_level_servers_emit_with_server_method() {
    let rendered = generate(indoc! {r##"
        openapi: 3.1.0
        info: { title: t, version: "0" }
        servers:
          - url: https://api.example.com
        paths:
          /ping:
            get:
              operationId: ping
              servers:
                - url: https://ping.example.com
                - url: https://alt-ping.example.com
              responses:
                "204":
                  description: ok
    "##});
    let compact = compact(&rendered);
    // Op-level server types collapse to fixed names (Server /
    // ServerOption{i}) inside the operation's own path module
    // (operations::ping::get in this case), same as Request / Response.
    assert!(
        compact.contains("pub mod ping") && compact.contains("pub mod get"),
        "op mod path missing:\n{rendered}"
    );
    assert!(
        compact.contains("pub struct ServerOption0"),
        "op-level ServerOption0 missing:\n{rendered}"
    );
    assert!(
        compact.contains("pub struct ServerOption1"),
        "op-level ServerOption1 missing:\n{rendered}"
    );
    assert!(
        compact.contains("pub enum Server"),
        "op-level aggregate missing:\n{rendered}"
    );
    assert!(
        compact.contains("pub fn with_server(self,") && compact.contains("server: Server"),
        "with_server method signature missing:\n{rendered}"
    );
    assert!(
        compact.contains("::toac::WithServer<Self>"),
        "with_server return type wrong:\n{rendered}"
    );
}
