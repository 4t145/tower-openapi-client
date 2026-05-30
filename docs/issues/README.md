# Known issues

Spec-handling bugs surfaced while building AI clients with `toac`.

The four issues below share one root: `toac` feeds the spec straight into
the `oas3` parser with no dialect normalization. OAS-3.0-isms and
oversized numeric literals either crash the parser or get silently
dropped *before* codegen runs — so they are **not** codegen bugs, and
cannot be fixed in the generator. They need a normalization pass that
rewrites the spec (text or value tree) before `oas3::from_*`.

The current per-app `patch_spec` text munging in
`examples/openai/build.rs` is exactly this normalization, done by hand in
the wrong layer. The plan is to lift it into a reusable pass.

| ID | Title | Layer | Status |
|----|-------|-------|--------|
| [B1](b1-oversized-integer-literals.md) | Oversized integer literals rejected by YAML parser | `oas3` / `yaml_serde` parse | open |
| [B2](b2-boolean-exclusive-min-max.md) | Draft-4 boolean `exclusiveMinimum`/`Maximum` rejected | `oas3` parse | open |
| [B3](b3-nullable-keyword-dropped.md) | OAS-3.0 `nullable: true` silently dropped | `oas3` parse (data loss) | open |
| [B5](b5-nullable-vec.md) | `nullable: true` on arrays dropped → required `Vec<T>` | `oas3` parse (data loss) | open |

B4 (discriminator tag clobbering) and B6 (non-deterministic map order)
were genuine codegen bugs and are already fixed in `toac-build`; they are
not tracked here.
