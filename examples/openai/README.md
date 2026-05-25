# OpenAI example

Generates an OpenAI client from the upstream
[`openai/openai-openapi`](https://github.com/openai/openai-openapi)
spec, vendored as a git submodule under
[`../openai-openapi`](../openai-openapi).

The submodule is marked `update = none` in `.gitmodules` so a normal
`git submodule update --init --recursive` skips it. Opt in explicitly
when you want to build this example:

```sh
git submodule update --init examples/openai-openapi
cargo build -p openai-example
```

Without the submodule the example will not build (the spec file is
absent), which is intentional — the rest of the workspace is happy to
compile and test without paying that cost.

The build script ([`build.rs`](build.rs)) patches a few non-conformant
literals in the upstream spec before handing it to `toac_build`; see the
inline comments for the rationale.
