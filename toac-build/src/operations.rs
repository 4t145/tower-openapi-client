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
    /// during the operations stage. Items declare their target sub-module
    /// through `Generator::set_mod_path`; this method groups them by
    /// that path and emits a nested `pub mod` tree.
    ///
    /// References to component types get rewritten to the absolute path
    /// `crate::components::Foo` — that works at any nesting depth and
    /// matches the `toac::include_client!` convention of placing
    /// generated code at the crate root.
    pub fn finish_operations(&self) -> proc_macro2::TokenStream {
        let order = match self.orders.get(&Stage::Operations) {
            Some(v) if !v.is_empty() => v,
            _ => {
                return quote! {
                    pub mod operations {}
                };
            }
        };

        let component_idents = self.component_type_idents();
        let operation_idents = self.operation_type_idents();
        let mut rewriter = QualifyComponents {
            component_idents,
            skip_idents: operation_idents,
        };

        // Group items by their mod path (empty path = operations root).
        let mut root = ModNode::default();
        for key in order {
            let Some(item) = self.items.get(key) else {
                continue;
            };
            let mut rewritten = item.clone();
            rewriter.visit_item_mut(&mut rewritten);
            let path = self.mod_path_for(key);
            root.insert(path, rewritten);
        }

        let body = root.render();
        quote! {
            pub mod operations {
                #body
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
/// component-level types with the absolute path `crate::components::...`.
///
/// Absolute form (instead of `super::...`) keeps the rewrite independent
/// of how deeply nested an operation's module ends up under
/// `operations::` — the path-based module tree can be several levels
/// deep, and computing the right number of `super::`s per item is more
/// fragile than relying on the `toac::include_client!` convention of
/// placing the generated module at the crate root.
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
        node.path = syn::parse_quote!(crate::components::#ident);
        if let Some(last) = node.path.segments.last_mut() {
            last.arguments = args;
        }
    }
}

/// Node in the in-memory tree that mirrors the nested `pub mod` shape
/// of the rendered `operations` module. Built up from each item's
/// `mod_path`, then `render`d out as tokens.
#[derive(Default)]
struct ModNode {
    /// Items placed directly at this module level, in registration order.
    items: Vec<syn::Item>,
    /// Child modules, keyed by segment name. Registration order is
    /// preserved through `children_order`.
    children: std::collections::BTreeMap<String, ModNode>,
    /// Order in which child mods were first seen, so emission is
    /// deterministic and mirrors walking order.
    children_order: Vec<String>,
}

impl ModNode {
    fn insert(&mut self, path: &[String], item: syn::Item) {
        match path.split_first() {
            None => self.items.push(item),
            Some((head, tail)) => {
                if !self.children.contains_key(head) {
                    self.children_order.push(head.clone());
                }
                self.children
                    .entry(head.clone())
                    .or_default()
                    .insert(tail, item);
            }
        }
    }

    fn render(&self) -> proc_macro2::TokenStream {
        let items = &self.items;
        let child_mods = self.children_order.iter().map(|name| {
            let child = &self.children[name];
            let body = child.render();
            let ident = syn::Ident::new(name, proc_macro2::Span::call_site());
            quote! {
                pub mod #ident {
                    #body
                }
            }
        });
        quote! {
            #(#items)*
            #(#child_mods)*
        }
    }
}
