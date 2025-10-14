use std::{
    cell::{LazyCell, OnceCell},
    collections::HashMap,
    sync::{LazyLock, OnceLock},
};

use http::Method;
use oas3::spec;
use quote::{ToTokens, quote};
use syn::{parse::Parse, parse_quote};
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
    parse_quote! { ::serde_json::Value }
}
pub fn json_never() -> syn::TypePath {
    parse_quote! { ::std::convert::Infallible }
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
            spec::Schema::Object(object_or_reference) => {
                self.resolve_or_build_object_or_reference(type_name, object_or_reference)
            }
        }
    }
    pub fn resolve_or_build_object_or_reference(
        &mut self,
        type_name: &str,
        object_or_reference: &spec::ObjectOrReference<spec::ObjectSchema>,
    ) -> Result<syn::TypePath, Error> {
        match object_or_reference {
            spec::ObjectOrReference::Ref { ref_path, .. } => {
                if let Some(ref_type) = self.built_types.get(ref_path) {
                    Ok(ref_type.type_path.clone())
                } else {
                    let schema = object_or_reference.resolve(&self.spec)?;
                    let built_type = self.build_schema_type(type_name, &schema)?;
                    self.built_types
                        .insert(ref_path.clone(), built_type.clone());
                    Ok(built_type.type_path)
                }
            }
            spec::ObjectOrReference::Object(obj) => {
                let built_type = self.build_schema_type(type_name, obj)?;
                self.built_types
                    .insert(type_name.to_owned(), built_type.clone());
                Ok(built_type.type_path)
            }
        }
    }
    pub fn build_schema_type(
        &mut self,
        type_name: &str,
        body: &spec::ObjectSchema,
    ) -> Result<BuiltType, Error> {
        let mut attributes: Vec<syn::Attribute> = vec![];
        let mut docs: Vec<String> = Vec::new();
        if let Some(true) = body.deprecated {
            attributes.push(parse_quote! { #[deprecated] });
        }
        if let Some(title) = &body.title {
            let line = format!("/// #{}", title);
            docs.push(line);
        }
        if let Some(description) = &body.description {
            for line in description.lines() {
                let doc_line = format!("/// {}", line);
                docs.push(doc_line);
            }
        }
        // example field is deprecated, just ignore it for now
        // if let Some(example) = &body.example { todo!("add example to document") }
        if !body.examples.is_empty() {
            docs.push("# Examples".to_owned());
        }
        for example in &body.examples {
            let example_str = serde_json::to_string_pretty(example).unwrap_or_default();
            docs.push("# Example".to_owned());
            docs.push("```json".to_owned());
            docs.extend(example_str.lines().map(|s| s.to_owned()));
            docs.push("```".to_owned());
        }
        for doc_line in docs {
            let doc_attrs: syn::Attribute = parse_quote!( #[doc = #doc_line] );
            attributes.push(doc_attrs);
        }
        if let Some(type_set) = &body.schema_type {
            let item = match type_set {
                spec::SchemaTypeSet::Multiple(types) => {}
                spec::SchemaTypeSet::Single(t) => {
                    let rust_type = match t {
                        spec::SchemaType::String => {
                            parse_quote! { pub type #type_name = String }
                        }
                        spec::SchemaType::Number => {
                            parse_quote! { pub type #type_name = f64 }
                        }
                        spec::SchemaType::Object => {
                            let mut fields = Vec::new();
                            for (field_name, field_type) in &body.properties {
                                let type_path = self
                                    .resolve_or_build_object_or_reference(field_name, field_type)?;
                                let field: syn::Field = parse_quote! {
                                    pub #field_name: #type_path
                                };
                                fields.push(field);
                            }
                            let definition = syn::ItemStruct {
                                attrs: attributes,
                                vis: parse_quote! { pub },
                                struct_token: Default::default(),
                                ident: syn::Ident::new(type_name, proc_macro2::Span::call_site()),
                                generics: Default::default(),
                                fields: syn::Fields::Named(syn::FieldsNamed {
                                    brace_token: Default::default(),
                                    named: syn::punctuated::Punctuated::from_iter(fields),
                                }),
                                semi_token: None,
                            };
                            syn::Item::Struct(definition)
                        }
                        spec::SchemaType::Array => {
                            // check items
                            match body.items.as_deref() {
                                Some(spec::Schema::Boolean(boolean_schema)) => {}
                                Some(spec::Schema::Object(object_reference)) => {
                                    let schema = object_reference.resolve(&self.spec)?;
                                }
                                None => todo!(),
                            }
                            parse_quote! { pub type #type_name = Vec<::serde_json::Value> }
                        }
                        spec::SchemaType::Boolean => {
                            parse_quote! { pub type #type_name = bool }
                        }
                        spec::SchemaType::Integer => {
                            parse_quote! { pub type #type_name = i64 }
                        }
                        spec::SchemaType::Null => {
                            parse_quote! { pub type #type_name = () }
                        }
                    };
                }
            }
        }
        todo!("Implement schema type building")
    }
}
