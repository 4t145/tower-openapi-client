use std::collections::HashMap;

use http::Method;
use oas3::spec;
use quote::{ToTokens, quote};
pub fn build_from_spec(spec: oas3::Spec) -> Result<(), spec::Error> {
    for (name, method, operation) in spec.operations() {
        let request_body = operation.request_body(&spec)?;
    }
    Ok(())
}

pub fn build_schema_type(
    spec: spec::Spec,
    body: &spec::ObjectSchema,
) -> Result<syn::Item, spec::Error> {
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ItemType {
    TypeDefinition { type_name: String },
}

pub struct Builder {
    spec: oas3::Spec,
    built_types: HashMap<ItemType, syn::Item>,
}

impl Builder {
    pub fn build_schema_type(
        &mut self,
        body: &spec::ObjectSchema,
    ) -> Result<(), spec::Error> {

    }
}
