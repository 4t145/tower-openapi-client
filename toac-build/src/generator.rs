//! Global code generator state.
//!
//! [`Generator`] threads through the entire codegen pipeline so that every
//! stage — components, paths, servers, tags — writes into the same type
//! registry and item list. This keeps cross-stage references (e.g. a path's
//! request type referencing a component schema) naturally consistent and
//! gives a single place to perform whole-program passes like the recursive
//! `Box` insertion.
//!
//! Schema-specific generation methods live in [`crate::components::schema`];
//! they are all defined as `impl Generator` blocks so the full surface is
//! reachable from one type.

use std::collections::BTreeMap;

use oas3::spec::{ObjectOrReference, ObjectSchema};
use syn::parse_quote;

use crate::{Error, naming::type_ident};

/// Prefix for every schema component ref path: `#/components/schemas/<Name>`.
pub(crate) const SCHEMA_REF_PREFIX: &str = "#/components/schemas/";

/// Registry key prefix for anonymous, hoisted schema types.
///
/// Hoisted types have no ref path, so we synthesise one with this prefix to
/// keep the registry keyed uniformly.
pub(crate) const HOIST_KEY_PREFIX: &str = "__hoist/";

/// Registry key prefix for secondary items (e.g. `From`/`TryFrom` impls)
/// emitted alongside a primary item.
pub(crate) const EXTRA_KEY_PREFIX: &str = "__extra/";

/// The generator pipeline stage whose items are currently being written.
///
/// Items get placed into the corresponding per-stage order list so that
/// `finish` can emit them into the right Rust module (`components`,
/// `operations`, ...). The shared item / type-path registries are global,
/// so cross-stage `$ref`s still resolve to the same generated Rust type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Default stage before any `emit_*` call — items end up in `components`.
    Components,
    /// Per-operation request/response types.
    Operations,
    /// Server options, aggregate server enum, and client alias.
    Servers,
}

/// Codegen tunables passed through to the generator.
///
/// Every flag here makes the generator emit a richer Rust type in place
/// of a plain `String`. The default has all flags off so that the
/// produced code compiles without adding new dependencies — callers
/// must add the corresponding crates themselves (`chrono`, `uuid`, or
/// this crate's own `base64` feature) when turning a flag on.
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    /// Emit `::chrono::DateTime<::chrono::Utc>` for `format: date-time`,
    /// `::chrono::NaiveDate` for `format: date`, and
    /// `::chrono::NaiveTime` for `format: time`.
    pub use_chrono: bool,

    /// Emit `::uuid::Uuid` for `format: uuid`.
    pub use_uuid: bool,

    /// Emit `::toac::Base64String` for `format: byte`. Requires the
    /// consumer to enable the `base64` feature of the `toac` runtime
    /// crate.
    pub use_base64_string: bool,
}

/// Whole-program state shared across every generation stage.
pub struct Generator<'a> {
    /// The OpenAPI spec being generated from.
    pub(crate) spec: &'a oas3::Spec,

    /// Codegen options selected by the caller.
    pub(crate) options: BuildOptions,

    /// Stable emission order per stage, keyed by registry key
    /// (ref path / hoist key / extra key).
    pub(crate) orders: BTreeMap<Stage, Vec<String>>,

    /// Registry key → the emitted Rust item. Global across stages.
    pub(crate) items: BTreeMap<String, syn::Item>,

    /// Registry key → the Rust type path that references that item. Hoist
    /// and ref entries populate this; `__extra/*` entries do not (they are
    /// impls, not named types). Global across stages.
    pub(crate) type_paths: BTreeMap<String, syn::Type>,

    /// Registry key → nested module path where the item should be emitted.
    /// An empty vec (or absent entry) means "top of the stage's module";
    /// `["pets", "by_id", "get"]` means `operations::pets::by_id::get::*`.
    /// Only the `Operations` stage populates this today.
    pub(crate) item_mod_paths: BTreeMap<String, Vec<String>>,

    /// Sticky mod path applied to every subsequent `store_*` call until
    /// cleared or replaced. Lets call sites emit a cluster of items
    /// (struct + impls + sub-types) into the same mod without threading
    /// the path through every helper.
    pub(crate) current_mod_path: Vec<String>,

    /// Stage currently receiving new items.
    pub(crate) current_stage: Stage,

    /// Monotonic counter used to disambiguate synthesised idents and
    /// registry keys.
    pub(crate) anon_counter: usize,
}

impl<'a> Generator<'a> {
    /// Creates an empty generator bound to `spec` with default options.
    pub fn new(spec: &'a oas3::Spec) -> Self {
        Self::with_options(spec, BuildOptions::default())
    }

    /// Creates an empty generator bound to `spec`, configured with the
    /// supplied [`BuildOptions`].
    pub fn with_options(spec: &'a oas3::Spec, options: BuildOptions) -> Self {
        let mut orders: BTreeMap<Stage, Vec<String>> = BTreeMap::new();
        orders.insert(Stage::Components, Vec::new());
        orders.insert(Stage::Operations, Vec::new());
        orders.insert(Stage::Servers, Vec::new());
        Self {
            spec,
            options,
            orders,
            items: BTreeMap::new(),
            type_paths: BTreeMap::new(),
            item_mod_paths: BTreeMap::new(),
            current_mod_path: Vec::new(),
            current_stage: Stage::Components,
            anon_counter: 0,
        }
    }

    /// Returns the active codegen options.
    pub fn options(&self) -> &BuildOptions {
        &self.options
    }

    /// Switches the active stage. Items emitted afterwards register under
    /// the new stage's order list.
    pub fn set_stage(&mut self, stage: Stage) {
        self.current_stage = stage;
    }

    /// Sets the sticky mod path for subsequent `store_*` calls.
    ///
    /// Every item registered after this call, until [`Self::clear_mod_path`]
    /// or another `set_mod_path`, records its module home as `path`. The
    /// stage's `finish_*` method uses those paths to build a nested
    /// `pub mod` tree instead of emitting one flat list.
    pub(crate) fn set_mod_path<P: Into<Vec<String>>>(&mut self, path: P) {
        self.current_mod_path = path.into();
    }

    /// Clears the sticky mod path so subsequent items land at the stage
    /// module's root.
    pub(crate) fn clear_mod_path(&mut self) {
        self.current_mod_path.clear();
    }

    /// Returns the mod path an item was registered under. Empty means
    /// "stage module root".
    pub(crate) fn mod_path_for(&self, key: &str) -> &[String] {
        self.item_mod_paths
            .get(key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the items emitted during `stage`, in registration order.
    pub fn items_in_stage(&self, stage: Stage) -> Vec<syn::Item> {
        self.orders
            .get(&stage)
            .into_iter()
            .flatten()
            .filter_map(|k| self.items.get(k).cloned())
            .collect()
    }

    /// Resolves an `ObjectOrReference` wrapper against the bound spec.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Ref`] if the `$ref` cannot be resolved within the
    /// spec.
    pub(crate) fn resolve(
        &self,
        schema_or_ref: &ObjectOrReference<ObjectSchema>,
    ) -> Result<ObjectSchema, Error> {
        Ok(schema_or_ref.resolve(self.spec)?)
    }

    /// Registers a hoisted (anonymous) type alongside its backing item.
    ///
    /// Used when an inline schema is rich enough to deserve its own name —
    /// the synthesised key prevents collisions with component ref paths.
    pub(crate) fn store_hoisted(&mut self, name: syn::Ident, item: syn::Item) {
        let key = format!("{HOIST_KEY_PREFIX}{name}");
        let ty: syn::Type = parse_quote!(#name);
        self.type_paths.insert(key.clone(), ty);
        self.items.insert(key.clone(), item);
        self.push_to_current(key);
    }

    /// Stores a named item under a generator-supplied registry key.
    ///
    /// Used by stages that own their own naming scheme (e.g. operations)
    /// and don't route through `ensure_schema`.
    pub(crate) fn store_named(&mut self, key: String, name: syn::Ident, item: syn::Item) {
        let ty: syn::Type = parse_quote!(#name);
        self.type_paths.insert(key.clone(), ty);
        self.items.insert(key.clone(), item);
        self.push_to_current(key);
    }

    /// Stores an item that carries no name of its own (e.g. an inherent
    /// `impl` block) under a synthesised registry key.
    pub(crate) fn store_unnamed(&mut self, item: syn::Item) {
        self.anon_counter += 1;
        let key = format!("{EXTRA_KEY_PREFIX}{}", self.anon_counter);
        self.items.insert(key.clone(), item);
        self.push_to_current(key);
    }

    /// Appends secondary items (e.g. conversion impls) whose identity is
    /// derived from a primary item. The key is synthesised so it never
    /// collides with a ref-path key.
    pub(crate) fn store_extra_items<I: IntoIterator<Item = syn::Item>>(&mut self, items: I) {
        for item in items {
            self.store_unnamed(item);
        }
    }

    fn push_to_current(&mut self, key: String) {
        if !self.current_mod_path.is_empty() {
            self.item_mod_paths
                .insert(key.clone(), self.current_mod_path.clone());
        }
        self.orders.entry(self.current_stage).or_default().push(key);
    }

    /// Records a ref-path-shaped registry key in the active stage's order.
    pub(crate) fn push_ref_path(&mut self, key: String) {
        self.push_to_current(key);
    }

    /// Synthesises a fresh type ident for an anonymous child schema.
    ///
    /// Preference order: `{Parent}{Field}` → `{Parent}{Field}{N}`. The
    /// chosen ident is guaranteed not to clash with any type already in
    /// the registry.
    pub(crate) fn hoist_name(&mut self, parent: &syn::Ident, field_name: &str) -> syn::Ident {
        let base = format!("{parent}{}", crate::naming::to_pascal_case(field_name));
        let candidate = type_ident(&base);
        if !self.type_path_exists(&candidate) {
            return candidate;
        }
        self.anon_counter += 1;
        let counter = self.anon_counter;
        type_ident(&format!("{base}{counter}"))
    }

    /// Returns `true` if a type with this ident has already been registered.
    pub(crate) fn type_path_exists(&self, ident: &syn::Ident) -> bool {
        let needle = ident.to_string();
        self.type_paths.values().any(|ty| match ty {
            syn::Type::Path(p) => p.path.get_ident().is_some_and(|segment| *segment == needle),
            _ => false,
        })
    }

    /// Returns the bound spec, for call sites that need direct access
    /// (e.g. walking path items outside the generator methods).
    pub fn spec(&self) -> &oas3::Spec {
        self.spec
    }
}
