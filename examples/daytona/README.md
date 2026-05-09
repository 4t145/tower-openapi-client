# Daytona example

End-to-end walkthrough driving the generated Daytona client against
the real API at `https://app.daytona.io/api`.

## Run

```sh
# Create a personal API key from the Daytona dashboard and export it.
export DAYTONA_API_KEY="dtn_..."
cargo run -p daytona-example
```

Without `DAYTONA_API_KEY` set the binary logs an error and exits — no
network traffic happens.

## What this demonstrates

- Code generation from a large, real-world 3.1 spec (201 paths, several
  hundred component schemas, mixed `bearer` / `oauth2` security schemes)
- Path-module layout — every operation lives under
  `operations::<path>::<method>::{Request, Response}`
- Authenticated dispatch through `ApiClient::with_auth` plus a hand-
  rolled `AuthSelector` that attaches a `BearerCredential`

Once the Security codegen lands the bespoke `AuthSelector` goes away
and the client is built with `.with_auth(AuthConfig::builder().bearer(...).build())`
instead.

## Spec source

The spec fixture at `fixtures/openapi.json` is a verbatim copy of the
document published at <https://www.daytona.io/docs/openapi.json>.
Refresh it with:

```sh
curl -sSL https://www.daytona.io/docs/openapi.json \
  -o examples/daytona/fixtures/openapi.json
```
