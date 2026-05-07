# Petstore Example

End-to-end walkthrough of using `toac` against a real
OpenAPI 3.1 specification — the Swagger Petstore sample at
[`tests/test_petstore31_json_spec/openapi.json`].

This example is a **workspace member but not a default-member**, so the
normal `cargo test` run skips it. Invoke it explicitly:

```sh
cargo run -p petstore-example
```

## What's demonstrated

| Scenario               | Code path                                                   |
|------------------------|-------------------------------------------------------------|
| Inspecting a request   | `GetPetByIdRequest::make_request()` + metadata consts  |
| 200 round-trip         | `ApiClient<_>::call(GetPetByIdRequest)` → typed `Pet`       |
| 404 branch             | Same op, returns `GetPetByIdResponse::Status404`            |
| POST with JSON body    | `AddPetRequest { body: Pet { ... } }` (serialised via serde)|
| PUT + enum field       | `UpdatePetRequest` round-trip, demoing `PetStatus::Sold`    |

The transport is a hand-written [`tower::Service`] that returns canned
responses — swap it out for a real HTTP adapter (e.g. reqwest wrapped in
an `http::Request → http::Response` tower service) to hit a live server.

Call sites use [`tower::ServiceExt::oneshot`] so a single request only
costs one line:

```rust
use tower::ServiceExt;
let resp = client.oneshot(GetPetByIdRequest { pet_id: 1 }).await?;
```

The request value pins down which `Service<Op>` impl is used, so no
turbofish is needed.

## How the integration is wired

Three files carry all the weight:

### `Cargo.toml`

The crate lists `toac-build` under `[build-dependencies]` (so
`build.rs` can call the code generator) and `toac` under
`[dependencies]` (so the generated code's `::toac::*` references
resolve). It also pulls in `chrono`, `uuid`, `bytes` — the crates the
richer format mappings reference — and enables `toac`'s `base64`
feature for `Base64String`.

### `build.rs`

```rust
fn main() {
    toac_build::Builder::new("path/to/openapi.yml")
        .use_chrono(true)
        .use_uuid(true)
        .use_base64_string(true)
        .emit();
}
```

The builder reads the spec, parses it (picking the right parser from
the extension), pretty-prints the output, and writes
`$OUT_DIR/<stem>.rs`. Pass `.output_file_name("something.rs")` to
override the derived name.

### `src/lib.rs`

```rust
toac::include_client!("openapi");  // pairs with `openapi.yml` → `openapi.rs`
```

That's it. The rest of `src/main.rs` is usage code — constructing
request values, plugging them into `ApiClient`, and decoding the
typed response.

## Known quirks hit by this spec

The Petstore 3.1 sample uses JSON Schema `$id` / `$anchor` pointers for a
couple of fields. `oas3` 0.21 doesn't surface those, so the generator
falls back to `serde_json::Value` with a documentation note. The
example sidesteps those fields; everything else round-trips through
typed Rust.

[`tests/test_petstore31_json_spec/openapi.json`]: ../../tests/test_petstore31_json_spec/openapi.json
[`tower::Service`]: https://docs.rs/tower
[`tower::ServiceExt::oneshot`]: https://docs.rs/tower/latest/tower/util/trait.ServiceExt.html#method.oneshot
