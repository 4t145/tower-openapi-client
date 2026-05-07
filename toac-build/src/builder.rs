//! Convenience API for calling the generator from a `build.rs`.
//!
//! [`Builder`] handles the boring parts — reading the spec file,
//! picking the right oas3 parser from the extension, pretty-printing
//! the output, and dropping the result into the crate's `OUT_DIR` —
//! so the caller's build script can stay focused on *which spec* and
//! *which options*.
//!
//! # Example
//!
//! ```no_run
//! # // (In a real build.rs this is inside `fn main() { ... }`.)
//! toac_build::Builder::new("openapi.yml")
//!     .use_chrono(true)
//!     .use_uuid(true)
//!     .emit();
//! ```
//!
//! The example above generates `$OUT_DIR/openapi.rs`. The sibling
//! `toac::include_client!("openapi")` macro on the runtime side picks
//! the file up by the same stem.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::{BuildOptions, build_with};

/// Source format for the spec file.
///
/// [`Builder`] auto-detects this from the file extension, but callers
/// can override when the extension is unusual.
#[derive(Debug, Clone, Copy)]
pub enum SpecFormat {
    /// JSON source (extensions: `.json`).
    Json,
    /// YAML source (extensions: `.yml`, `.yaml`).
    Yaml,
}

/// Build-script helper that turns an OpenAPI spec into a Rust module
/// in `$OUT_DIR`.
///
/// See the [module-level documentation][self] for the typical usage
/// pattern. Every builder method takes `self` by value to keep call
/// sites in the canonical method-chain form.
pub struct Builder {
    spec_path: PathBuf,
    options: BuildOptions,
    output_file_name: Option<String>,
    format: Option<SpecFormat>,
}

impl Builder {
    /// Starts a build targeting `spec_path`. The path is resolved
    /// relative to the crate's manifest directory (the default CWD for
    /// build scripts), matching `cargo:rerun-if-changed` semantics.
    pub fn new(spec_path: impl AsRef<Path>) -> Self {
        Self {
            spec_path: spec_path.as_ref().to_path_buf(),
            options: BuildOptions::default(),
            output_file_name: None,
            format: None,
        }
    }

    /// Enables `format: date-time / date / time` → `::chrono::...`
    /// mappings. See [`BuildOptions::use_chrono`].
    pub fn use_chrono(mut self, enabled: bool) -> Self {
        self.options.use_chrono = enabled;
        self
    }

    /// Enables `format: uuid` → `::uuid::Uuid`.
    pub fn use_uuid(mut self, enabled: bool) -> Self {
        self.options.use_uuid = enabled;
        self
    }

    /// Enables `format: byte` → `::toac::Base64String`. Consumers must
    /// also enable the `base64` feature on the `toac` runtime crate.
    pub fn use_base64_string(mut self, enabled: bool) -> Self {
        self.options.use_base64_string = enabled;
        self
    }

    /// Replaces the full [`BuildOptions`]. Chain this when you have a
    /// preconfigured value rather than setting individual flags.
    pub fn options(mut self, options: BuildOptions) -> Self {
        self.options = options;
        self
    }

    /// Overrides the generated file name inside `$OUT_DIR`. Defaults
    /// to `<spec-stem>.rs` — e.g. `openapi.yml` → `openapi.rs`.
    pub fn output_file_name(mut self, name: impl Into<String>) -> Self {
        self.output_file_name = Some(name.into());
        self
    }

    /// Forces a specific input format. By default the extension picks
    /// [`SpecFormat::Yaml`] for `.yml`/`.yaml` and [`SpecFormat::Json`]
    /// for `.json`.
    pub fn format(mut self, format: SpecFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// Runs the generator and writes the output under `$OUT_DIR`.
    ///
    /// # Panics
    ///
    /// Panics with a descriptive message on any of:
    /// - `OUT_DIR` not set (i.e. not running inside a build script);
    /// - spec file missing or unreadable;
    /// - spec format can't be inferred from the extension;
    /// - spec failing to parse;
    /// - generator rejecting the spec;
    /// - writing the generated file failing.
    ///
    /// build scripts surface panics through cargo's build-error
    /// reporting, so `Result`-returning control flow isn't useful
    /// here.
    pub fn emit(self) {
        let out_dir = env::var_os("OUT_DIR")
            .expect("Builder::emit must run inside a build script (OUT_DIR is unset)");
        self.run_to(Path::new(&out_dir))
            .unwrap_or_else(|err| panic!("toac-build: {err}"));
    }

    fn run_to(self, out_dir: &Path) -> Result<PathBuf, BuilderError> {
        let Builder {
            spec_path,
            options,
            output_file_name,
            format,
        } = self;

        let abs_spec = spec_path.clone();
        emit_rerun_directive(&abs_spec);

        let format = format
            .or_else(|| infer_format(&abs_spec))
            .ok_or_else(|| BuilderError::UnknownFormat(abs_spec.clone()))?;

        let contents = fs::read_to_string(&abs_spec)
            .map_err(|e| BuilderError::ReadSpec(abs_spec.clone(), e))?;

        let spec = match format {
            SpecFormat::Json => oas3::from_json(&contents)
                .map_err(|e| BuilderError::ParseSpec(abs_spec.clone(), Box::new(e)))?,
            SpecFormat::Yaml => oas3::from_yaml(&contents)
                .map_err(|e| BuilderError::ParseSpec(abs_spec.clone(), Box::new(e)))?,
        };

        let tokens = build_with(&spec, options).map_err(BuilderError::Codegen)?;

        // Pretty-print so compile errors in generated code come with
        // readable line numbers.
        let parsed: syn::File = syn::parse_file(&tokens.to_string())
            .map_err(|e| BuilderError::GeneratedInvalid(Box::new(e)))?;
        let rendered = prettyplease::unparse(&parsed);

        let output_name = output_file_name.unwrap_or_else(|| default_output_name(&abs_spec));
        let out_path = out_dir.join(&output_name);
        fs::write(&out_path, rendered)
            .map_err(|e| BuilderError::WriteOutput(out_path.clone(), e))?;
        Ok(out_path)
    }
}

/// Returns the default output file name for a given spec path.
///
/// Uses the spec's file stem with a `.rs` extension: `pets.yaml` →
/// `pets.rs`. Falls back to `generated.rs` only when the input has no
/// usable stem.
fn default_output_name(spec_path: &Path) -> String {
    spec_path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| format!("{s}.rs"))
        .unwrap_or_else(|| "generated.rs".to_owned())
}

/// Prints the `cargo:rerun-if-changed=<path>` directive for the spec
/// file, so edits to it trigger a rebuild.
fn emit_rerun_directive(path: &Path) {
    if let Some(p) = path.to_str() {
        println!("cargo:rerun-if-changed={p}");
    }
}

/// Picks a [`SpecFormat`] from the path's extension.
fn infer_format(path: &Path) -> Option<SpecFormat> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "json" => Some(SpecFormat::Json),
        "yaml" | "yml" => Some(SpecFormat::Yaml),
        _ => None,
    }
}

/// Error variants surfaced by [`Builder::emit`] as a panic message.
///
/// Stored separately so [`Builder::run_to`] can stay testable while
/// `emit` still offers a one-line panic-on-error surface.
#[derive(Debug, thiserror::Error)]
enum BuilderError {
    #[error("could not infer spec format from extension of {0}; call `.format(...)` explicitly")]
    UnknownFormat(PathBuf),

    #[error("failed to read spec at {0}: {1}")]
    ReadSpec(PathBuf, #[source] std::io::Error),

    #[error("failed to parse spec at {0}: {1}")]
    ParseSpec(PathBuf, #[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("codegen failed: {0}")]
    Codegen(#[source] crate::Error),

    #[error("generator produced invalid Rust: {0}")]
    GeneratedInvalid(#[source] Box<syn::Error>),

    #[error("failed to write generated output to {0}: {1}")]
    WriteOutput(PathBuf, #[source] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_name_uses_stem() {
        assert_eq!(default_output_name(Path::new("openapi.yml")), "openapi.rs");
        assert_eq!(default_output_name(Path::new("pets.json")), "pets.rs");
        assert_eq!(
            default_output_name(Path::new("dir/petstore.yaml")),
            "petstore.rs"
        );
    }

    #[test]
    fn infer_format_accepts_common_extensions() {
        assert!(matches!(
            infer_format(Path::new("x.json")),
            Some(SpecFormat::Json)
        ));
        assert!(matches!(
            infer_format(Path::new("x.yml")),
            Some(SpecFormat::Yaml)
        ));
        assert!(matches!(
            infer_format(Path::new("x.YAML")),
            Some(SpecFormat::Yaml)
        ));
        assert!(infer_format(Path::new("x.toml")).is_none());
        assert!(infer_format(Path::new("noext")).is_none());
    }
}
