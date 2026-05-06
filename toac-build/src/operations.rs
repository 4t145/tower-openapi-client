//! Generator entry points for the `paths` section of an OpenAPI spec.
//!
//! Every path / HTTP-method pair becomes an operation with a dedicated
//! request type and response enum, plus the inherent metadata (method,
//! path template) needed by the runtime layer.

pub mod operation;

use std::collections::BTreeSet;

use quote::quote;
use syn::visit_mut::VisitMut;

use crate::{
    Error, Generator,
    generator::{HOIST_KEY_PREFIX, SCHEMA_REF_PREFIX, Stage},
};

impl<'a> Generator<'a> {
    /// Walks `spec.paths` and emits a request type + response enum per
    /// operation.
    ///
    /// # Errors
    ///
    /// Propagates per-operation generation errors.
    pub fn emit_operations(&mut self) -> Result<(), Error> {
        self.set_stage(Stage::Operations);
        let Some(paths) = self.spec.paths.as_ref() else {
            return Ok(());
        };
        // Clone so the mutable loop can call `&mut self` methods without
        // borrowing `self.spec`.
        let paths: Vec<(String, oas3::spec::PathItem)> = paths
            .iter()
            .map(|(p, item)| (p.clone(), item.clone()))
            .collect();
        for (path, item) in paths {
            self.emit_path_item(&path, &item)?;
        }
        Ok(())
    }

    /// Renders `pub mod operations { ... }` from the items registered
    /// during the operations stage, prefixing bare references to
    /// component types with `super::components::` so the generated module
    /// compiles.
    pub fn finish_operations(&self) -> proc_macro2::TokenStream {
        let mut items = self.items_in_stage(Stage::Operations);
        if items.is_empty() {
            return quote! {
                pub mod operations {}
            };
        }

        let component_idents = self.component_type_idents();
        let operation_idents = self.operation_type_idents();
        let mut rewriter = QualifyComponents {
            component_idents,
            skip_idents: operation_idents,
        };
        for item in items.iter_mut() {
            rewriter.visit_item_mut(item);
        }

        quote! {
            pub mod operations {
                #(#items)*
            }
        }
    }

    /// Collects the Rust type idents registered by the components stage —
    /// i.e. anything whose registry key identifies a component schema or
    /// a hoisted helper type emitted during that stage.
    fn component_type_idents(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let Some(order) = self.orders.get(&Stage::Components) else {
            return out;
        };
        for key in order {
            let Some(ty) = self.type_paths.get(key) else {
                continue;
            };
            if let syn::Type::Path(p) = ty
                && let Some(ident) = p.path.get_ident()
            {
                out.insert(ident.to_string());
            }
            // Silences unused-import warnings for the prefix constants;
            // future stages may use them directly.
            let _ = (SCHEMA_REF_PREFIX, HOIST_KEY_PREFIX);
        }
        out
    }

    /// Collects the Rust type idents registered by the operations stage
    /// so the rewriter skips them (a `GetPetRequest` referenced inside
    /// the operations module must stay local).
    fn operation_type_idents(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let Some(order) = self.orders.get(&Stage::Operations) else {
            return out;
        };
        for key in order {
            let Some(ty) = self.type_paths.get(key) else {
                continue;
            };
            if let syn::Type::Path(p) = ty
                && let Some(ident) = p.path.get_ident()
            {
                out.insert(ident.to_string());
            }
        }
        out
    }
}

/// `syn::visit_mut` visitor that qualifies bare references to
/// component-level types with `super::components::...` so they resolve
/// from inside the `operations` module.
struct QualifyComponents {
    component_idents: BTreeSet<String>,
    skip_idents: BTreeSet<String>,
}

impl VisitMut for QualifyComponents {
    fn visit_type_path_mut(&mut self, node: &mut syn::TypePath) {
        // Recurse into generic arguments first so inner types get
        // rewritten too.
        if let Some(last) = node.path.segments.last_mut()
            && let syn::PathArguments::AngleBracketed(args) = &mut last.arguments
        {
            for arg in args.args.iter_mut() {
                if let syn::GenericArgument::Type(t) = arg {
                    self.visit_type_mut(t);
                }
            }
        }

        // Only rewrite bare, single-segment idents (leading `::` or
        // multi-segment paths are already qualified).
        if node.qself.is_some() || node.path.leading_colon.is_some() {
            return;
        }
        if node.path.segments.len() != 1 {
            return;
        }
        let segment = &node.path.segments[0];
        let ident_str = segment.ident.to_string();
        if !self.component_idents.contains(&ident_str) {
            return;
        }
        if self.skip_idents.contains(&ident_str) {
            return;
        }
        let ident = segment.ident.clone();
        let args = segment.arguments.clone();
        node.path = syn::parse_quote!(super::components::#ident);
        if let Some(last) = node.path.segments.last_mut() {
            last.arguments = args;
        }
    }
}
