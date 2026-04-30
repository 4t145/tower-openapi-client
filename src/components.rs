//! Generator entry points for the `components` section of an OpenAPI spec.
//!
//! The current implementation covers `components/schemas`. Other component
//! kinds (responses, parameters, request bodies, ...) are staged for later
//! work.

pub mod schema;

use quote::quote;

use crate::{Error, Generator, generator::Stage};

impl<'a> Generator<'a> {
    /// Registers every schema in `spec.components.schemas` into the
    /// generator, materialising their Rust items and linking cross-schema
    /// `$ref`s through the shared registry.
    ///
    /// # Errors
    ///
    /// Propagates [`Error::Ref`] or [`Error::Unsupported`] from the
    /// per-schema generation logic.
    pub fn emit_components(&mut self) -> Result<(), Error> {
        self.set_stage(Stage::Components);
        let Some(components) = self.spec.components.as_ref() else {
            return Ok(());
        };
        // Collect to avoid borrowing `self.spec` across the mutable loop.
        let schemas: Vec<(
            String,
            oas3::spec::ObjectOrReference<oas3::spec::ObjectSchema>,
        )> = components
            .schemas
            .iter()
            .map(|(name, schema_or_ref)| (name.clone(), schema_or_ref.clone()))
            .collect();
        for (name, schema_or_ref) in schemas {
            self.ensure_schema(&name, &schema_or_ref)?;
        }
        Ok(())
    }

    /// Renders a `pub mod components { ... }` from the items registered
    /// during the components stage, applying whole-module passes
    /// (`Box`-insertion for recursive types) along the way.
    pub fn finish_components(&self) -> proc_macro2::TokenStream {
        let mut items = self.items_in_stage(Stage::Components);
        schema::box_recursive_cycles(&mut items);
        quote! {
            pub mod components {
                #(#items)*
            }
        }
    }
}
