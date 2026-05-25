# toac — Tower-compatible OpenAPI Client

[![CI](https://github.com/4t145/tower-openapi-client/actions/workflows/ci.yml/badge.svg)](https://github.com/4t145/tower-openapi-client/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/toac.svg)](https://crates.io/crates/toac)
[![docs.rs](https://docs.rs/toac/badge.svg)](https://docs.rs/toac)
[![License](https://img.shields.io/crates/l/toac.svg)](#license)

`toac` (**T**ower **O**pen**A**PI **C**lient) generates a Tower-native
HTTP client straight from an [OpenAPI 3.x] specification. The generated
code links against a small runtime (`toac`) and plugs into any
transport that speaks `tower::Service<http::Request<_>>` — `hyper`,
`reqwest`, `tower-http`, `wiremock`, your in-memory test stub, anything.

[OpenAPI 3.x]: https://spec.openapis.org/oas/latest.html

```text
                ┌──────────────────┐                ┌─────────────────┐
   build.rs ──▶ │  toac-build      │ ──── emits ──▶ │  Generated Rust │
                │  (code generator)│                │  client module  │
                └──────────────────┘                └────────┬────────┘
                                                             │ links to
                                                             ▼
                                                    ┌─────────────────┐
                                                    │      toac       │
                                                    │  (runtime lib)  │
                                                    └────────┬────────┘
                                                             │ wraps
                                                             ▼
                                                    ┌─────────────────┐
                                                    │ tower::Service  │
                                                    │ (hyper/reqwest) │
                                                    └─────────────────┘
```

## What you get

- **Typed `Request` / `Response`** for every operation, organised by
  URL path: `GET /pets/{id}` becomes
  `operations::pets::by_id::get::{Request, Response}`.
- **Single-generic `ApiClient<S>`** that accepts any transport speaking
  `Service<http::Request<Body>, Response = http::Response<B>>`.
- **First-class auth.** API key (header / query / cookie), HTTP Bearer,
  and HTTP Basic out of the box, with a generated `AuthConfig` builder
  per spec. OAuth2 / mTLS hooks are reserved (see [`TODO.md`](TODO.md)).
- **Codecs that match real specs.** JSON (incl. `+json` vendor types),
  `application/x-www-form-urlencoded`, `multipart/form-data`,
  `application/octet-stream`, `text/plain` are always on; XML, NDJSON,
  and SSE behind cargo features.
- **Servers as types.** Each `servers[]` entry is a generated struct;
  variables surface as fields. Pick one at construction time, or
  override per request via `WithServer`.
- **Components as Rust types.** `components.schemas` map to structs /
  enums / type aliases with `serde` derives — including
  `discriminator` → internally-tagged enums, `oneOf`/`anyOf` →
  untagged enums, recursive types auto-`Box`'d, etc.

## Quick start

Add the runtime and the build-time generator to your crate:

```toml
# Cargo.toml
[build-dependencies]
toac-build = "0.1"

[dependencies]
toac = "0.1"
http = "1"
http-body-util = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tower = { version = "0.5", features = ["util"] }
```

Drop the spec next to your crate (e.g. `petstore.yml`) and have
`build.rs` generate the client into `OUT_DIR`:

```rust
// build.rs
fn main() {
    toac_build::Builder::new("petstore.yml").emit();
}
```

Include the generated module from your crate:

```rust
// src/lib.rs
toac::include_client!("petstore");
```

Now use it:

```rust
use tower::ServiceExt;

let transport = /* any tower::Service<toac::Request> */;
let client = toac::ApiClient::new(transport, servers::ServerOption0);

let resp = client
    .oneshot(operations::pets::by_id::get::Request { pet_id: 1 })
    .await?;
```

A complete walkthrough — including a real HTTPS round-trip — lives in
[`examples/petstore`](examples/petstore).

## Cargo features (`toac`)

| Feature   | Purpose                                                       |
|-----------|---------------------------------------------------------------|
| `base64`  | `Base64String` for `format: byte` (required when codegen sets `use_base64_string = true`) |
| `xml`     | `application/xml` / `text/xml` codec (via `quick-xml`)        |
| `ndjson`  | Streaming decoder for `application/x-ndjson` / `jsonl`        |
| `sse`     | Streaming decoder for `text/event-stream`                     |
| `reqwest` | Adapter that turns `reqwest::Client` into a `tower::Service`  |

## `toac-build` knobs

`toac_build::Builder` is a tiny façade over the generator. The common
toggles:

```rust
toac_build::Builder::new("openapi.yml")
    .output_file_name("openapi.rs")  // default: <stem>.rs in $OUT_DIR
    .use_chrono(true)                // map date/date-time → chrono types
    .use_uuid(true)                  // map uuid → uuid::Uuid
    .use_base64_string(true)         // map format: byte → toac::Base64String
    .emit();
```

## Examples

The `examples/` directory hosts opt-in workspace members built against
real-world specs:

| Example       | Spec source                                                  |
|---------------|--------------------------------------------------------------|
| `petstore`    | The OpenAPI 3.1 Pet Store sample (vendored)                  |
| `daytona`     | Daytona public API (vendored under `fixtures/`)              |
| `openai`      | [`openai/openai-openapi`](https://github.com/openai/openai-openapi) — opt-in submodule |
| `github`      | [`github/rest-api-description`](https://github.com/github/rest-api-description) — opt-in submodule |

The OpenAI and GitHub specs live in heavy git submodules with
`update = none`, so a default `git submodule update --init --recursive`
*skips* them. Opt in explicitly when you want to build those examples:

```sh
git submodule update --init examples/openai-openapi
cargo build -p openai-example
```

## Status

`0.1.x`. The runtime + codegen surface is settled enough to publish; a
few rough edges and feature gaps are tracked in [`TODO.md`](TODO.md).
Breaking changes will land in `0.2.x` and beyond.

### Branching & releases

- `master` is the development branch — PRs land here.
- `release` is the publishing branch — only commits that are ready to
  ship are merged in (typically a bump-version + changelog commit).
- Versioned crates are released by pushing a `v*` tag pointing at a
  commit on `release`. The release workflow refuses tags whose commit
  is not reachable from `origin/release`.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms
or conditions.
