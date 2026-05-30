# B3 — OAS-3.0 `nullable: true` silently dropped

## Symptom

A `3.1.0` spec that still uses the OAS-3.0 `nullable: true` keyword on a
property produces a non-`Option` field. A `required` + `nullable` field
(e.g. a streaming `finish_reason` that is present but may be `null`)
becomes `T` instead of `Option<T>`, so deserializing a `null` value
fails upstream.

## Layer

`oas3` deserialization — **silent data loss**, before any `toac` codegen.

## Evidence

Reproduced against `oas3` 0.22 with a `3.1.0` header and a
`required: [reason]` property `reason: { type: string, nullable: true }`:

```text
B3 reason.is_nullable() = Some(false)
B3 reason.schema_type   = Some(Single(String))
B3 reason.extensions    = Map { inner: {} }
```

`oas3` 0.22 models nullability the JSON-Schema 2020-12 way — via the
`type` set (`type: [string, "null"]`). It does not recognize the OAS-3.0
`nullable` keyword: the field is dropped during deserialization and is
not even preserved under `extensions` (it is not an `x-` key).

`ObjectSchema::is_nullable()` only inspects the `type` set, so it returns
`Some(false)` and the generator never learns the field is nullable.

## Why this is not a codegen bug

`toac`'s `maybe_optionalise` / `build_field` logic is correct: given a
schema that `is_nullable()`, it wraps the field in `Option<_>`. It simply
never sees the nullability because `oas3` discarded it at parse time. The
`type: [string, "null"]` form already works end-to-end in `toac` today
(see `enum_contains_null` / `non_null_type` handling).

## Current workaround

Apps remove the affected field from `required` (or post-process the
generated code) so the field becomes `Option<T>`. Per-app and brittle.

## Root fix (planned)

In the normalization pass, rewrite the OAS-3.0 `nullable: true` shape
into the OAS-3.1 type-set form before `oas3` parses it:

```yaml
# before
reason: { type: string, nullable: true }
# after
reason: { type: [string, "null"] }
```

and strip the `nullable` key. This is the single highest-value
normalization since `nullable` is pervasive in 3.0-authored specs that
declare `openapi: 3.1.0`. A YAML/JSON value-tree pass handles the
type → type-set restructuring cleanly; pure text replacement is brittle
here because of YAML indentation sensitivity.

See also [B5](b5-nullable-vec.md) (same root, array-typed fields).
