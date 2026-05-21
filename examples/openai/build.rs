//! Generates the OpenAI client from the upstream OpenAPI spec vendored
//! as a git submodule under `../openai-openapi`. The submodule tracks
//! `https://github.com/openai/openai-openapi`; refresh it with
//! `git submodule update --remote examples/openai-openapi` to pick up
//! upstream changes.
//!
//! The upstream spec ships with `seed: { minimum: -9.22e18, maximum:
//! 9.22e18 }` literals that fall just outside `i64::MIN..=i64::MAX`,
//! which the `oas3` parser rejects. We patch those two numeric bounds
//! down to `i64::MIN` / `i64::MAX` in a vendored copy under `OUT_DIR`
//! before handing the file to the generator.

use std::{env, fs, path::PathBuf};

const UPSTREAM_SPEC: &str = "../openai-openapi/openapi.yaml";
const PATCHED_SPEC_NAME: &str = "openai-patched.yaml";

/// Numeric bound the upstream spec writes as a 19-digit literal that
/// overflows `i64` by 193 — the parser rejects it. We round to the
/// representable max/min, which is what every existing client does.
const UPSTREAM_OUT_OF_RANGE_LIT: &str = "9223372036854776000";
const I64_MAX_LIT: &str = "9223372036854775807";

fn main() {
    println!("cargo:rerun-if-changed={UPSTREAM_SPEC}");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR not set"));
    let patched = out_dir.join(PATCHED_SPEC_NAME);
    let raw = fs::read_to_string(UPSTREAM_SPEC).expect("read upstream OpenAI spec");
    let fixed = patch_spec(&raw);
    fs::write(&patched, fixed).expect("write patched spec");

    toac_build::Builder::new(&patched)
        .output_file_name("openai.rs")
        .use_chrono(true)
        .use_uuid(true)
        .use_base64_string(true)
        .emit();
}

/// Applies the small fixups the upstream spec needs to round-trip
/// through the strict `oas3` parser. Each substitution is local and
/// doesn't change the API surface, so the generated client stays
/// faithful to the published OpenAI shape.
fn patch_spec(raw: &str) -> String {
    let mut s = raw.replace(UPSTREAM_OUT_OF_RANGE_LIT, I64_MAX_LIT);
    // Normalise stray snake_case forms of OAS keywords. The official
    // spelling is camelCase; the upstream YAML has a couple of typos.
    for (from, to) in [("min_items: ", "minItems: "), ("max_items: ", "maxItems: ")] {
        s = s.replace(from, to);
    }
    // OAS 3.1 / JSON Schema 2020-12 expects `exclusiveMinimum` /
    // `exclusiveMaximum` as numbers, not booleans (the OAS 3.0 shape).
    // The upstream spec is mixed on this — strip the boolean lines so
    // the parser keeps the surrounding `minimum`/`maximum` constraints.
    // Codegen doesn't use these bounds, so dropping them is safe.
    s = strip_lines(&s, |line| {
        let t = line.trim();
        t == "exclusiveMinimum: true" || t == "exclusiveMaximum: true"
    });
    s
}

fn strip_lines(input: &str, drop: impl Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.split_inclusive('\n') {
        if !drop(line.trim_end_matches(['\n', '\r'])) {
            out.push_str(line);
        }
    }
    out
}
