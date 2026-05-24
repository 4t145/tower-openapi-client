//! Generator entry points for the `paths` section of an OpenAPI spec.
//!
//! Every path / HTTP-method pair becomes an operation with a dedicated
//! request type and response enum, plus the inherent metadata (method,
//! path template) needed by the runtime layer.

pub mod operation;

use quote::quote;

use crate::{Error, Generator, attrs::module_inner_attrs, generator::Stage};

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
    /// References to component types are already absolute
    /// (`crate::components::Foo`) when they leave `inline_type` during
    /// the operations stage, so this method does no path rewriting.
    /// Absolute form is required because op-private idents like
    /// `Response` can shadow component names of the same spelling — see
    /// `Generator::qualify_for_current_stage`.
    pub fn finish_operations(&self) -> proc_macro2::TokenStream {
        let order = match self.orders.get(&Stage::Operations) {
            Some(v) if !v.is_empty() => v,
            _ => {
                return quote! {
                    pub mod operations {}
                };
            }
        };

        let mut root = ModNode::default();
        for key in order {
            let Some(item) = self.items.get(key) else {
                continue;
            };
            let path = self.mod_path_for(key);
            root.insert(path, item.clone());
        }

        let body = root.render();
        let attrs = module_inner_attrs();
        quote! {
            pub mod operations {
                #attrs
                #body
            }
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
            // `make_ident` handles Rust keywords by emitting the
            // `r#` raw-identifier form (or a `_`-suffix fallback for
            // the few keywords that can't be raw-escaped), which lets
            // paths like `/type` and `/move` round-trip into legal mod
            // names.
            let ident = crate::naming::make_ident(name);
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
