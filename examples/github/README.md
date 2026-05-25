# GitHub example

Generates a GitHub REST client from the upstream
[`github/rest-api-description`](https://github.com/github/rest-api-description)
spec, vendored as a git submodule under
[`../github-openapi`](../github-openapi).

The submodule is **large** (>100 MB packed) and is marked
`update = none` in `.gitmodules` so a normal
`git submodule update --init --recursive` skips it. Opt in explicitly
when you want to build this example:

```sh
git submodule update --init examples/github-openapi
cargo build -p github-example
```

Without the submodule the example will not build (the spec file is
absent), which is intentional — the rest of the workspace is happy to
compile and test without paying that cost.
