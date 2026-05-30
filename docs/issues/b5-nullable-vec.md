# B5 — `nullable: true` on arrays dropped → required `Vec<T>`

## Symptom

An array property carrying OAS-3.0 `nullable: true` is generated as a
required `Vec<T>` instead of an optional / defaultable one. A wire `null`
(or an absent field that may be `null`) then fails to deserialize.

```yaml
items:
  type: array
  nullable: true
  items: { type: string }
```

## Layer

`oas3` deserialization — same root as [B3](b3-nullable-keyword-dropped.md):
the `nullable` keyword is dropped at parse time. Before any `toac`
codegen.

## Why this is not a codegen bug

Once the array schema is nullable in the type set
(`type: [array, "null"]`), `toac` already wraps the field correctly:
`inline_object_type` applies `maybe_optionalise(Vec<T>, is_nullable)` →
`Option<Vec<T>>`. The generator just never sees the nullability because
`oas3` discarded it.

## Current workaround

`examples/anthropic` (and similar) add `#[serde(default)]` to the
generated `Vec<T>` field by hand / post-processing, so an absent or null
array deserializes to an empty vec. Per-app and brittle.

## Root fix (planned)

Covered by the same `nullable` → type-set rewrite as
[B3](b3-nullable-keyword-dropped.md). No array-specific handling is
needed once the keyword is normalized before parsing — `toac` already
produces `Option<Vec<T>>` from `type: [array, "null"]`.

Design choice to settle during the normalization work: whether nullable
collections should map to `Option<Vec<T>>` (strict) or to `Vec<T>` with
`#[serde(default)]` (lenient, treats null/absent as empty). The latter
matches the current hand-applied workaround.
