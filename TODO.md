# TODO

Status snapshot — 2026-05-25.

For background on big completed efforts, see the design notes under
[`docs/design/`](docs/design/).

## ✅ Shipped (0.1.0)

- **Runtime core**: `MakeRequest` / `ParseResponse` / `Operation` /
  single-generic `ApiClient<S>`.
- **Servers**: `Server` trait + blanket impls (`&str` / `String` /
  `Arc<str>`) + `WithServer` op-level override; generator emits
  `pub mod servers` with `ServerOption*` + aggregate `ApiServer`.
- **Security (P0)**: `SecurityCredential` / `AuthSelector` /
  `OperationSecurity` / `NoAuth` + built-in API Key / Bearer / Basic;
  generated `pub mod security` with `AuthConfig` + builder;
  `ApiClient::with_auth`. See [docs/design/security.md](docs/design/security.md).
- **Codecs**: JSON (incl. `+json`), `application/x-www-form-urlencoded`,
  `multipart/form-data`, `application/octet-stream`, `text/plain`
  always-on. XML (`xml` feature), NDJSON (`ndjson`), SSE (`sse`)
  feature-gated.
- **Multi-codec response dispatch**: when a single status declares
  several content types (`application/json` + `text/event-stream`,
  ...), distinct variants are emitted (`Status200Json`, `Status200Sse`,
  ...). Wire selection via `Request::with_accept(...)` at runtime.
- **`CallError`**: `Encode` / `Auth` / `Transport` / `Decode`.
- **Operations org**: types live under
  `operations::<path>::<method>::{Request, Response, Server}`. See
  [docs/design/path-mod-reorg.md](docs/design/path-mod-reorg.md).
- **Components**: `components.schemas` → Rust types (recursive auto-`Box`,
  `allOf` flattening, `oneOf`/`anyOf` → enums, `discriminator` →
  internally-tagged serde, string enums with `Display`).
- **Spec metadata**: `info` / `externalDocs` / `license` / `contact` /
  root `description` rendered as a doc-block on a generated
  `pub mod spec {}`.
- **Builder API**: `toac_build::Builder` handles read / parse / pretty /
  write end-to-end so consumers' `build.rs` is a one-liner.
- **Backend adapters**: `toac::compat::reqwest` (behind the `reqwest`
  feature) turns a `reqwest::Client` into a
  `tower::Service<toac::Request>`.
- **Examples**: `petstore` (real HTTPS via hyper), `daytona`
  (`AuthConfig`-driven bearer auth), `openai` (SSE round-trip,
  opt-in submodule), `github` (large 3.1 spec, opt-in submodule).

## ☐ Planned but not started

### `ClientExt` convenience layer

Generate `pub trait ClientExt<S>` + blanket impl on
`toac::ApiClient<S>`, with one `operationId`-named method per op so
callers can write `client.get_pet(req).await` instead of
`client.oneshot(req).await`. Path mods are already in place — see
[docs/design/client-ext.md](docs/design/client-ext.md) for the full
design note (typestate builders via `bon` opt-in, naming rules,
multi-spec disambiguation).

### Crate-level doc on the generated module

Spec metadata currently lands on `pub mod spec {}`. Some downstream
crates would prefer it as a real `#![doc = ...]` on the including
module. Needs a small `BuildOptions` toggle and a different emission
shape (inner attributes only legal at module head).

## 🐛 Known codegen quirks

### B5 — `oas3` parser is strict on OAS 3.0 / mixed shapes

Not a `toac` codegen bug, but it bites real specs:

- Numeric literals exceeding `i64` (OpenAI: `seed.minimum/maximum:
  ±9.22e18`) → `oas3` rejects with `invalid type: integer ... as i128`.
- `exclusiveMinimum: true` / `exclusiveMaximum: true` (OAS 3.0 boolean
  shape) → `oas3` (in 3.1 mode) reports `data did not match any variant
  of untagged enum ObjectOrReference`.
- `min_items` / `max_items` snake_case typos in upstream specs → same
  symptom.

`examples/openai/build.rs` patches its spec inline; longer-term we may
upstream an `oas3` PR or ship a "lenient parse" preprocessing layer.

### Missing schema features

- `prefixItems` (OAS 3.1 / JSON Schema 2020-12) not yet handled.
- OAS 3.0 `nullable: true` is not surfaced by `oas3` — fields in 3.0
  specs with this keyword come out as non-`Option`. Workaround: use
  the 3.1 `type: [T, 'null']` shape.

## 🚫 Out of scope

- Webhooks / Callbacks (server-side concern).
- Server codegen (this project is client-only).
- OAuth2 flow implementation — the trait surface (`SecurityCredential`
  is async + `Result`) leaves room for a future credential type, but
  shipping a token-refresh state machine is its own project.
- OpenID Connect Discovery — same reasoning.
- mutualTLS certificate management — transport-layer concern, not HTTP
  header injection.

## 🤔 Undecided

- **tag → mod grouping**: deferred. Currently tags are spec metadata
  only; if/when a spec is large enough to need navigation, we'll add
  it as a `ClientExt` entry point rather than restructuring types.
- **Non-`schemas` components** (`parameters` / `responses` /
  `requestBodies` / `headers` / `examples` / `links` / `callbacks` /
  `pathItems`): currently expanded inline via `oas3::Spec::resolve`.
  Whether to emit standalone Rust types is undecided.

## 📦 Dependency gaps (`oas3` 0.22)

These OpenAPI 3.1 / JSON Schema 2020-12 fields aren't surfaced by the
`oas3` crate, so `toac` doesn't see them. If upstream lands them, the
matching codegen passes can be enabled.

### `ObjectSchema`

- **`$id`** — schema identity URI; needed to resolve `$ref` to absolute
  identifiers (petstore3.1 `PetDetails { "$id": "/api/v31/..." }`).
  Currently degrades to `serde_json::Value` with a doc comment.
- **`$anchor`** — fragment anchors (`$ref: "....#pet_details_id"`).
  Same fallback.
- **`$ref` siblings** — in `{"$ref": "...", "type": "integer", "description": "..."}`,
  the sibling `type` / `description` / `format` are dropped because
  `oas3::ObjectOrReference` is `#[serde(untagged)]`.
- **`contains` / `maxContains` / `minContains`** — array containment.
- **`not`** — schema negation.
- **`if` / `then` / `else` / `dependentRequired` / `dependentSchemas`** —
  conditional sub-schemas.
- **`patternProperties` / `propertyNames` / `unevaluatedProperties` /
  `unevaluatedItems`** — object/array constraint extras.
- **`$defs`** — local sub-schema definitions.
- **`$schema` / `$vocabulary` / `$dynamicRef` / `$dynamicAnchor`** —
  vocabulary + dynamic refs.

### Extensions

- **`x-enum-varnames`** — de-facto standard for naming enum variants.
  `oas3` *does* expose this via `extensions`; only the codegen side is
  pending.

### Other

- **`Discriminator.mapping` external URIs** — `oas3` keeps mapping
  values as raw strings; cross-document URIs would need our own
  resolver but we don't reach for them today.
