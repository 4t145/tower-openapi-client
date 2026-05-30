# B1 — Oversized integer literals rejected by the YAML parser

## Symptom

A spec with an integer bound outside `i64` range (e.g. OpenAI's `seed`
field, `minimum: -9223372036854776000`, `maximum: 9223372036854776000`)
fails to parse through `oas3::from_yaml`.

## Layer

`oas3` / `yaml_serde` deserialization — happens before any `toac` codegen.

## Evidence

Reproduced against `oas3` 0.22:

```text
B1 JSON: OK
B1 YAML ERR: components.schemas.S.minimum: invalid type: integer
  `-9223372036854776000` as i128, expected any value
```

JSON input is accepted (`serde_json` stores the value as `f64`). YAML
input is rejected: `yaml_serde` hands the integer to `serde_json::Number`
as an i128, and `Number`'s visitor has no i128 → f64 fallback, so it
errors out.

The fields involved (`minimum`/`maximum`/`exclusiveMinimum`/
`exclusiveMaximum`/`multipleOf`) are `Option<serde_json::Number>` in
`oas3::spec::ObjectSchema`. The codegen never reads them, so the bound
itself is irrelevant to output — the only problem is parse rejection.

## Current workaround

`examples/openai/build.rs::patch_spec` text-replaces the out-of-range
literal `9223372036854776000` with `i64::MAX` (`9223372036854775807`)
before parsing. Per-app and fragile (matches the literal as a bare
string, no numeric-context awareness).

## Root fix (planned)

A normalization pass before `oas3::from_*` that clamps numeric
constraint values outside the representable range to the nearest
representable bound. Because codegen ignores these bounds, clamping is
behaviorally safe.

For the YAML path specifically, parsing into a `yaml_serde::Value` tree
first preserves the i128 so the pass can inspect and rewrite it before it
reaches the strict `serde_json::Number` visitor.
