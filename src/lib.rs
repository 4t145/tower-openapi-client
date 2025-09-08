use std::{cell::{LazyCell, OnceCell}, collections::HashMap, sync::{LazyLock, OnceLock}};

use http::Method;
use oas3::spec;
use quote::{ToTokens, quote};
use syn::parse::Parse;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("OpenAPI Specification error: {0}")]
    Spec(#[from] spec::Error),
    #[error("Syn error: {0}")]
    Syn(#[from] syn::Error),
    #[error("Reference error: {0}")]
    Ref(#[from] spec::RefError),
}

pub fn build_from_spec(spec: oas3::Spec) -> Result<(), spec::Error> {
    for (name, method, operation) in spec.operations() {
        let request_body = operation.request_body(&spec)?;
    }
    Ok(())
}

// 
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ItemType {
    TypeDefinition { ref_path: String },
}

pub struct Builder {
    spec: oas3::Spec,
    built_types: HashMap<String, BuiltType>,
}
#[derive(Clone)]
pub struct BuiltType {
    item: syn::Item,
    type_path: syn::TypePath,
}

pub fn json_any() -> syn::TypePath {
    syn::parse_quote! { ::serde_json::Value }
}
pub fn json_never() -> syn::TypePath {
    syn::parse_quote! { ::std::convert::Infallible }
}
impl Builder {
    pub fn resolve_or_build_type(
        &mut self,
        type_name: &str,
        schema: &spec::Schema,
    ) -> Result<syn::TypePath, Error> {
        match schema {
            spec::Schema::Boolean(spec::BooleanSchema(true)) => Ok(json_any()),
            spec::Schema::Boolean(spec::BooleanSchema(false)) => Ok(json_never()),
            spec::Schema::Object(object_reference) => {
                match object_reference.as_ref() {
                    spec::ObjectOrReference::Ref { ref_path, summary, description } => {
                        if let Some(ref_type) = self.built_types.get(ref_path) {
                            Ok(ref_type.type_path.clone())
                        }  else {
                            let schema = object_reference.resolve(&self.spec)?;
                            let built_type = self.build_schema_type(type_name, &schema)?;
                            self.built_types.insert(ref_path.clone(), built_type.clone());
                            Ok(built_type.type_path)
                        }
                    },
                    spec::ObjectOrReference::Object(obj) => {
                        let built_type = self.build_schema_type(type_name, obj)?;
                        self.built_types.insert(type_name.to_owned(), built_type.clone());
                        Ok(built_type.type_path)
                    },
                }
            }
        }
    }
    pub fn build_schema_type(
        &mut self,
        type_name: &str,
        body: &spec::ObjectSchema,
    ) -> Result<BuiltType, spec::Error> {
        if let Some(type_set) = body.schema_type {
            match type_set {
                spec::SchemaTypeSet::Multiple(types) => {

                }
                spec::SchemaTypeSet::Single(t) => {
                    let rust_type = match t {
                        spec::SchemaType::String => {
                            quote! { String }
                        }
                        spec::SchemaType::Number => {
                            quote! { f64 }
                        }
                        spec::SchemaType::Object => {
                            quote! { ::std::collections::HashMap<String, ::serde_json::Value> }
                        }
                        spec::SchemaType::Array => {
                            // check items
                            match body.items.as_deref() {
                                Some(spec::Schema::Boolean(boolean_schema)) => {

                                },
                                Some(spec::Schema::Object(object_reference)) => {
                                    let schema = object_reference.resolve(&self.spec)?;
                                },
                                None => todo!(),
                            }
                            quote! { Vec<::serde_json::Value> }
                        }
                        spec::SchemaType::Boolean => {
                            quote! { bool }
                        }
                        spec::SchemaType::Integer => {
                            quote! { i64 }
                        }
                        spec::SchemaType::Null => {
                            quote! { () } 
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
