# B2 — Draft-4 boolean `exclusiveMinimum`/`exclusiveMaximum` rejected

## Symptom

A schema using the JSON-Schema draft-4 / OAS-3.0 boolean form of
`exclusiveMinimum` / `exclusiveMaximum` makes the **entire schema** fail
to parse — not just the offending field.

```yaml
S:
  type: number
  minimum: 0
  exclusiveMinimum: true   # draft-4 boolean form
```

## Layer

`oas3` deserialization — before any `toac` codegen.

## Evidence

Reproduced against `oas3` 0.22:

```text
B2 YAML ERR: components.schemas: data did not match any variant of
  untagged enum Schema at line 8 column 5
```

In `oas3` 0.22 (JSON-Schema 2020-12 model), `exclusive_minimum` /
`exclusive_maximum` are typed `Option<serde_json::Number>`. A boolean
value fails the field, which fails the whole untagged `Schema` enum, so
the schema is dropped wholesale. The surrounding `minimum`/`maximum`
constraints are lost too.

## Current workaround

`examples/openai/build.rs::patch_spec` strips the lines
`exclusiveMinimum: true` / `exclusiveMaximum: true` before parsing,
keeping the inclusive `minimum`/`maximum` next to them. Comment notes the
upstream spec is mixed on this (some use the number form, some the bool
form). Codegen doesn't read these bounds, so dropping them is safe.

## Root fix (planned)

In the normalization pass, detect the draft-4 boolean form and either:
- drop the `exclusiveMinimum`/`exclusiveMaximum` key when its value is a
  boolean (simplest, matches the current workaround), or
- convert it to the 2020-12 form: `exclusiveMinimum: true` + `minimum: X`
  → `exclusiveMinimum: X` (only meaningful if a sibling `minimum` exists;
  more faithful but unused by codegen).

Dropping is sufficient given codegen ignores the bound.
