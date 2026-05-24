//! Conversion from `oas3::spec::ObjectSchema` to Rust items.
//!
//! The generator walks every referenced schema, ensuring that each named
//! component maps to exactly one Rust item, and that inline schemas are
//! either inlined as Rust primitives/containers or, when necessary, hoisted
//! into auxiliary named types.

use std::collections::BTreeMap;

use oas3::spec::{
    self, Discriminator, ObjectOrReference, ObjectSchema, Schema, SchemaType, SchemaTypeSet,
};
use quote::{ToTokens, quote};
use syn::parse_quote;

use crate::{
    Error,
    docs::{deprecated_attr, push_field_docs, push_schema_docs},
    generator::{Generator, SCHEMA_REF_PREFIX},
    naming::{field_ident, make_ident, type_ident},
};

/// Projects a top-level [`Schema`] (post-resolution) onto its
/// [`ObjectSchema`] payload. Boolean schemas (`true` / `false`) collapse
/// to the empty `ObjectSchema` because the generator treats them as the
/// "any value" / "no value" fallback elsewhere — keeping callers
/// uniform avoids threading the boolean-vs-object distinction through
/// every helper that consumed the old `ObjectOrReference<ObjectSchema>`
/// shape.
pub(crate) fn schema_to_object(schema: &Schema) -> ObjectSchema {
    match schema {
        Schema::Boolean(_) => ObjectSchema::default(),
        Schema::Object(boxed) => match boxed.as_ref() {
            ObjectOrReference::Object(inner) => inner.clone(),
            ObjectOrReference::Ref {
                ref_path,
                summary,
                description,
            } => ObjectSchema {
                // Builds a synthetic `Ref` schema so `flatten_all_of` and
                // friends can resolve through it without losing the
                // pointer.
                description: description.clone().or_else(|| summary.clone()),
                title: Some(format!("$ref: {ref_path}")),
                ..ObjectSchema::default()
            },
        },
    }
}

/// Visibility tokens applied to all generated items.
fn visibility() -> syn::Visibility {
    parse_quote!(pub)
}

impl<'a> Generator<'a> {
    /// Ensures the top-level component named `name` has been generated.
    ///
    /// # Errors
    ///
    /// Propagates resolution errors from `$ref` walking and generator errors
    /// for unsupported constructs.
    pub fn ensure_schema(&mut self, name: &str, schema: &Schema) -> Result<syn::Type, Error> {
        let ref_path = format!("{SCHEMA_REF_PREFIX}{name}");
        self.ensure_named_schema(&ref_path, name, schema)
    }

    /// Pre-registers a component schema's Rust type identifier so that
    /// later `hoist_name` calls can't collide with it. The reservation
    /// is tracked separately so [`Self::ensure_named_schema`] knows it
    /// still needs to build the underlying item on first real visit.
    /// Idempotent.
    pub(crate) fn reserve_schema_ident(&mut self, name: &str) {
        let ref_path = format!("{SCHEMA_REF_PREFIX}{name}");
        if self.type_paths.contains_key(&ref_path) {
            return;
        }
        let type_name = type_ident(name);
        let ty: syn::Type = parse_quote!(#type_name);
        self.type_paths.insert(ref_path.clone(), ty);
        self.reserved_refs.insert(ref_path);
    }

    /// Materialises a named schema, emitting it into `items` on first
    /// visit and returning the cached type path on subsequent visits.
    ///
    /// Two states are distinguished:
    /// - fully built (entry in both `type_paths` and `reserved_refs`
    ///   cleared) — early return with the cached type.
    /// - reserved placeholder (from [`Self::reserve_schema_ident`]:
    ///   entry in `type_paths` but still in `reserved_refs`) — fall
    ///   through to actually build, clearing the reservation.
    /// - recursing on a schema already in flight (entry in `type_paths`
    ///   but neither built nor reserved-pending) — early return so
    ///   self-references don't blow the stack.
    pub(crate) fn ensure_named_schema(
        &mut self,
        ref_path: &str,
        display_name: &str,
        schema: &Schema,
    ) -> Result<syn::Type, Error> {
        if let Some(ty) = self.type_paths.get(ref_path) {
            // A reservation means we haven't actually emitted yet — we
            // still need to build. Anything else (in-flight recursion
            // or fully built) short-circuits on the cached type.
            if !self.reserved_refs.contains(ref_path) {
                return Ok(ty.clone());
            }
        }

        let type_name = type_ident(display_name);
        let ty: syn::Type = parse_quote!(#type_name);
        self.type_paths.insert(ref_path.to_owned(), ty.clone());
        // Consume the reservation so recursive calls land in the
        // "in-flight" branch above instead of re-entering this
        // function.
        self.reserved_refs.remove(ref_path);

        let resolved = self.resolve_schema(schema)?;
        let item = self.build_named_item(&type_name, &resolved)?;
        self.items.insert(ref_path.to_owned(), item);
        self.push_ref_path(ref_path.to_owned());

        Ok(ty)
    }

    /// Emits the Rust item backing a named schema.
    fn build_named_item(
        &mut self,
        type_name: &syn::Ident,
        schema: &ObjectSchema,
    ) -> Result<syn::Item, Error> {
        if !schema.enum_values.is_empty() && is_string_schema(schema) {
            let mut items = build_string_enum_items(type_name, schema, doc_attrs(schema))?;
            // First item is the enum itself — return that as the
            // "named" component, store the rest (Display impl) as
            // extras so they land in the same module.
            let enum_item = items.remove(0);
            self.store_extra_items(items);
            return Ok(enum_item);
        }

        if let Some(sum) = detect_sum_kind(schema) {
            let items = self.build_sum_enum(type_name, schema, sum)?;
            self.store_extra_items(items[1..].iter().cloned());
            return Ok(items
                .into_iter()
                .next()
                .expect("sum produced at least the enum item"));
        }

        if let Some(merged) = flatten_all_of(schema, self.spec) {
            return self.build_named_item(type_name, &merged);
        }

        let attrs = struct_attrs(schema);

        match classify(schema) {
            Shape::Object => self.build_struct(type_name, schema, attrs),
            Shape::TypedPrimitive(ty) => {
                let alias_attrs = doc_attrs(schema);
                let ty = self.apply_format(*ty, schema);
                Ok(parse_quote! {
                    #(#alias_attrs)*
                    pub type #type_name = #ty;
                })
            }
            Shape::Array => {
                let alias_attrs = doc_attrs(schema);
                let item_ty = self.array_item_type(type_name, schema)?;
                Ok(parse_quote! {
                    #(#alias_attrs)*
                    pub type #type_name = Vec<#item_ty>;
                })
            }
            Shape::Map => {
                let alias_attrs = doc_attrs(schema);
                let value_ty = self.map_value_type(type_name, schema)?;
                Ok(parse_quote! {
                    #(#alias_attrs)*
                    pub type #type_name = ::std::collections::HashMap<String, #value_ty>;
                })
            }
            Shape::Fallback => {
                let mut alias_attrs = doc_attrs(schema);
                alias_attrs.push(
                    parse_quote!(#[doc = " Fallback: spec did not identify a concrete shape."]),
                );
                let any = json_any();
                Ok(parse_quote! {
                    #(#alias_attrs)*
                    pub type #type_name = #any;
                })
            }
        }
    }

    /// Builds a struct from an `object` schema, recursively materialising
    /// each field type.
    fn build_struct(
        &mut self,
        type_name: &syn::Ident,
        schema: &ObjectSchema,
        attrs: Vec<syn::Attribute>,
    ) -> Result<syn::Item, Error> {
        let mut fields: Vec<syn::Field> = Vec::with_capacity(schema.properties.len());
        for (field_name, field_schema) in &schema.properties {
            let field = self.build_field(type_name, field_name, field_schema, &schema.required)?;
            fields.push(field);
        }

        Ok(parse_quote! {
            #(#attrs)*
            pub struct #type_name {
                #(#fields,)*
            }
        })
    }

    fn build_field(
        &mut self,
        parent_type: &syn::Ident,
        field_name: &str,
        field_schema: &Schema,
        required: &[String],
    ) -> Result<syn::Field, Error> {
        let is_required = required.iter().any(|r| r == field_name);

        let (inner_type, inner_docs) = self.inline_type(parent_type, field_name, field_schema)?;
        let ty = if is_required {
            inner_type
        } else {
            parse_quote!(Option<#inner_type>)
        };

        let ident = field_ident(field_name);
        let mut attrs: Vec<syn::Attribute> = Vec::new();
        let rename = serde_rename_attr(field_name, &ident);
        attrs.extend(rename);
        if !is_required {
            attrs.push(parse_quote!(#[serde(skip_serializing_if = "Option::is_none")]));
        }
        push_field_docs(&mut attrs, inner_docs.description.as_deref());
        if let Some(dep) = inner_docs.deprecated {
            attrs.extend(dep);
        }

        Ok(syn::Field {
            attrs,
            vis: visibility(),
            mutability: syn::FieldMutability::None,
            ident: Some(ident),
            colon_token: Some(Default::default()),
            ty,
        })
    }

    /// Resolves an inline field schema to a Rust type. Hoists a named type
    /// when the schema is too rich to inline.
    pub(crate) fn inline_type(
        &mut self,
        parent_type: &syn::Ident,
        field_name: &str,
        field_schema: &Schema,
    ) -> Result<(syn::Type, InlineDocs), Error> {
        match field_schema {
            // `true` allows any value, `false` allows none. Both fall
            // back to `serde_json::Value` here — see `array_item_type_inline`
            // for the reasoning around `false`.
            Schema::Boolean(_) => Ok((json_any(), InlineDocs::default())),
            Schema::Object(boxed) => self.inline_type_or_ref(parent_type, field_name, boxed),
        }
    }

    /// Same as [`Self::inline_type`] but takes the inner
    /// `ObjectOrReference<ObjectSchema>` directly. Used by helpers that
    /// already hold the borrowed reference (e.g. allOf single-ref
    /// shortcut, sum-variant resolution).
    pub(crate) fn inline_type_or_ref(
        &mut self,
        parent_type: &syn::Ident,
        field_name: &str,
        field_schema: &ObjectOrReference<ObjectSchema>,
    ) -> Result<(syn::Type, InlineDocs), Error> {
        if let ObjectOrReference::Ref { ref_path, .. } = field_schema {
            match ref_name(ref_path) {
                Some(name) => {
                    let wrapped = Schema::Object(Box::new(field_schema.clone()));
                    let ty = self.ensure_named_schema(ref_path, name, &wrapped)?;
                    let ty = self.qualify_for_current_stage(ty);
                    return Ok((ty, InlineDocs::default()));
                }
                None => {
                    let docs = InlineDocs::unresolvable_ref(ref_path);
                    return Ok((json_any(), docs));
                }
            }
        }

        let ObjectOrReference::Object(schema) = field_schema else {
            unreachable!("matched Ref above");
        };

        let docs = InlineDocs::from_schema(schema);

        let ty = self.inline_object_type(parent_type, field_name, schema)?;
        Ok((ty, docs))
    }

    /// Promotes a bare component-typed `Foo` to
    /// `<root>::components::Foo` when we're emitting code outside the
    /// components module. The `<root>` prefix comes from
    /// [`BuildOptions::root_path`] so consumers can mount the generated
    /// tokens anywhere in their module tree. Inside the components
    /// stage itself, the bare form stays — both because that matches
    /// the surrounding mod path and because the recursive-box pass
    /// inspects bare idents.
    ///
    /// The qualification matters in operation modules where a generated
    /// `pub enum Response` shadows a `components::Response`: a bare
    /// `Status200(Response)` would resolve back to the enum and produce
    /// an infinite-size recursive type.
    fn qualify_for_current_stage(&self, ty: syn::Type) -> syn::Type {
        if self.current_stage != crate::generator::Stage::Operations {
            return ty;
        }
        let syn::Type::Path(path) = &ty else {
            return ty;
        };
        let Some(ident) = path.path.get_ident().cloned() else {
            return ty;
        };
        let root = &self.options.root_path;
        parse_quote!(#root::components::#ident)
    }

    fn inline_object_type(
        &mut self,
        parent_type: &syn::Ident,
        field_name: &str,
        schema: &ObjectSchema,
    ) -> Result<syn::Type, Error> {
        if let Some(alias) = allof_single_ref(schema) {
            let (ty, _) = self.inline_type(parent_type, field_name, alias)?;
            return Ok(maybe_optionalise(ty, is_nullable(schema)));
        }

        if let Some(merged) = flatten_all_of(schema, self.spec) {
            let ty = self.inline_object_type(parent_type, field_name, &merged)?;
            return Ok(maybe_optionalise(ty, is_nullable(schema)));
        }

        if !schema.enum_values.is_empty() && is_string_schema(schema) {
            let type_name = self.hoist_name(parent_type, field_name);
            let mut items = build_string_enum_items(&type_name, schema, doc_attrs(schema))?;
            let enum_item = items.remove(0);
            self.store_hoisted(type_name.clone(), enum_item);
            self.store_extra_items(items);
            let ty: syn::Type = parse_quote!(#type_name);
            return Ok(maybe_optionalise(ty, is_nullable(schema)));
        }

        if let Some(sum) = detect_sum_kind(schema) {
            let type_name = self.hoist_name(parent_type, field_name);
            let items = self.build_sum_enum(&type_name, schema, sum)?;
            let mut items_iter = items.into_iter();
            let enum_item = items_iter
                .next()
                .expect("sum produced at least the enum item");
            self.store_hoisted(type_name.clone(), enum_item);
            self.store_extra_items(items_iter);
            let ty: syn::Type = parse_quote!(#type_name);
            return Ok(maybe_optionalise(ty, is_nullable(schema)));
        }

        let shape = classify(schema);
        let ty = match shape {
            Shape::Object => {
                let type_name = self.hoist_name(parent_type, field_name);
                let attrs = struct_attrs(schema);
                let item = self.build_struct(&type_name, schema, attrs)?;
                self.store_hoisted(type_name.clone(), item);
                parse_quote!(#type_name)
            }
            Shape::TypedPrimitive(ty) => self.apply_format(*ty, schema),
            Shape::Array => {
                let item_ty = self.array_item_type_inline(parent_type, field_name, schema)?;
                parse_quote!(Vec<#item_ty>)
            }
            Shape::Map => {
                let value_ty = self.map_value_type_inline(parent_type, field_name, schema)?;
                parse_quote!(::std::collections::HashMap<String, #value_ty>)
            }
            Shape::Fallback => json_any(),
        };

        Ok(maybe_optionalise(ty, is_nullable(schema)))
    }

    fn array_item_type(
        &mut self,
        parent: &syn::Ident,
        schema: &ObjectSchema,
    ) -> Result<syn::Type, Error> {
        self.array_item_type_inline(parent, "item", schema)
    }

    fn array_item_type_inline(
        &mut self,
        parent: &syn::Ident,
        field_name: &str,
        schema: &ObjectSchema,
    ) -> Result<syn::Type, Error> {
        match schema.items.as_deref() {
            Some(Schema::Boolean(spec::BooleanSchema(true))) | None => Ok(json_any()),
            // `items: false` means "no items allowed" — the array must
            // be empty on the wire. The element type is unreachable, but
            // it still has to satisfy serde's bounds, so pick a serde-
            // compatible bottom-ish type rather than `Infallible`
            // (which has no `Serialize` / `Deserialize`).
            Some(Schema::Boolean(spec::BooleanSchema(false))) => Ok(json_any()),
            Some(Schema::Object(inner)) => {
                let (ty, _) = self.inline_type_or_ref(parent, field_name, inner.as_ref())?;
                Ok(ty)
            }
        }
    }

    fn map_value_type(
        &mut self,
        parent: &syn::Ident,
        schema: &ObjectSchema,
    ) -> Result<syn::Type, Error> {
        self.map_value_type_inline(parent, "value", schema)
    }

    fn map_value_type_inline(
        &mut self,
        parent: &syn::Ident,
        field_name: &str,
        schema: &ObjectSchema,
    ) -> Result<syn::Type, Error> {
        match &schema.additional_properties {
            None => Ok(json_any()),
            Some(Schema::Boolean(spec::BooleanSchema(true))) => Ok(json_any()),
            // `additionalProperties: false` means the map carries no
            // extra entries beyond declared properties. Same reasoning
            // as `items: false` above: the value type is unreachable,
            // but we still need a serde-compatible placeholder.
            Some(Schema::Boolean(spec::BooleanSchema(false))) => Ok(json_any()),
            Some(Schema::Object(inner)) => {
                let (ty, _) = self.inline_type_or_ref(parent, field_name, inner.as_ref())?;
                Ok(ty)
            }
        }
    }

    /// Builds an enum from `oneOf` / `anyOf`, returning the enum as the first
    /// item followed by zero or more `From`/`TryFrom` impls.
    fn build_sum_enum(
        &mut self,
        type_name: &syn::Ident,
        schema: &ObjectSchema,
        kind: SumKind,
    ) -> Result<Vec<syn::Item>, Error> {
        let members = sum_members(schema);
        let mut variants: Vec<SumVariant> = Vec::with_capacity(members.len());
        let mut used_variant_names: Vec<String> = Vec::with_capacity(members.len());

        for (index, member) in members.iter().enumerate() {
            let variant =
                self.build_sum_variant(type_name, index, member, &kind, &mut used_variant_names)?;
            variants.push(variant);
        }

        let enum_attrs = sum_enum_attrs(schema, &kind);
        let variant_tokens: Vec<proc_macro2::TokenStream> = variants
            .iter()
            .map(|v| {
                let ident = &v.ident;
                let ty = &v.inner_type;
                let rename = v.serde_rename.as_ref().map(|name| {
                    quote! { #[serde(rename = #name)] }
                });
                quote! {
                    #rename
                    #ident(#ty)
                }
            })
            .collect();

        let enum_item: syn::Item = parse_quote! {
            #(#enum_attrs)*
            pub enum #type_name {
                #(#variant_tokens,)*
            }
        };

        let mut out: Vec<syn::Item> = Vec::with_capacity(1 + variants.len() * 2);
        out.push(enum_item);

        let mut seen_inner_types: BTreeMap<String, ()> = BTreeMap::new();
        for variant in &variants {
            let key = variant.inner_type.to_token_stream().to_string();
            if seen_inner_types.insert(key, ()).is_some() {
                // Duplicate inner type: skip From/TryFrom to avoid
                // conflicting impls.
                continue;
            }
            out.extend(sum_conversion_impls(type_name, variant));
        }

        if let Some(display_impl) = sum_display_impl(type_name, &variants) {
            out.push(display_impl);
        }

        Ok(out)
    }

    /// Resolves one `oneOf`/`anyOf` member into a variant definition.
    fn build_sum_variant(
        &mut self,
        parent_type: &syn::Ident,
        index: usize,
        member: &Schema,
        kind: &SumKind,
        used_names: &mut Vec<String>,
    ) -> Result<SumVariant, Error> {
        let (inner_type, ref_name_opt) = match member {
            Schema::Boolean(_) => (json_any(), None),
            Schema::Object(boxed) => match boxed.as_ref() {
                ObjectOrReference::Ref { ref_path, .. } => match ref_name(ref_path) {
                    Some(name) => {
                        let name = name.to_owned();
                        let ty = self.ensure_named_schema(ref_path, &name, member)?;
                        let ty = self.qualify_for_current_stage(ty);
                        (ty, Some(name))
                    }
                    None => (json_any(), None),
                },
                ObjectOrReference::Object(schema) => {
                    let field_name = format!("variant_{index}");
                    let ty = self.inline_object_type(parent_type, &field_name, schema)?;
                    (ty, None)
                }
            },
        };

        let (ident, serde_rename) = sum_variant_ident(
            parent_type,
            index,
            kind,
            ref_name_opt.as_deref(),
            used_names,
        );
        Ok(SumVariant {
            ident,
            inner_type,
            serde_rename,
        })
    }
}

/// Extracts the component name from a `#/components/schemas/...` pointer.
///
/// Returns `None` for any other `$ref` shape (cross-document refs,
/// JSON-Schema-style absolute URIs backed by `$id`, etc.). The `oas3` crate
/// does not surface the metadata needed to resolve those to a component,
/// so callers treat them as opaque and fall back to `serde_json::Value`.
fn ref_name(ref_path: &str) -> Option<&str> {
    ref_path.strip_prefix(SCHEMA_REF_PREFIX)
}

/// Coarse classification of an inline schema into a single generation path.
enum Shape {
    /// Concrete object with properties.
    Object,
    /// Primitive value (string/number/integer/boolean/null).
    TypedPrimitive(Box<syn::Type>),
    /// Array with an item schema.
    Array,
    /// Open map with `additionalProperties`.
    Map,
    /// No discriminating information; treat as `serde_json::Value`.
    Fallback,
}

fn classify(schema: &ObjectSchema) -> Shape {
    match non_null_type(schema) {
        Some(SchemaType::Array) => Shape::Array,
        Some(SchemaType::Object) => {
            if schema.properties.is_empty() && schema.additional_properties.is_some() {
                Shape::Map
            } else {
                Shape::Object
            }
        }
        Some(prim) => match primitive_type(&prim) {
            Some(ty) => Shape::TypedPrimitive(Box::new(ty)),
            None => Shape::Fallback,
        },
        None => {
            if !schema.properties.is_empty() {
                Shape::Object
            } else if schema.additional_properties.is_some() {
                Shape::Map
            } else if schema.items.is_some() {
                Shape::Array
            } else {
                Shape::Fallback
            }
        }
    }
}

/// Returns the primitive-ish [`SchemaType`] of a schema, stripping a
/// `null` companion if present.
fn non_null_type(schema: &ObjectSchema) -> Option<SchemaType> {
    match schema.schema_type.as_ref()? {
        SchemaTypeSet::Single(SchemaType::Null) => None,
        SchemaTypeSet::Single(t) => Some(*t),
        SchemaTypeSet::Multiple(types) => types.iter().copied().find(|t| *t != SchemaType::Null),
    }
}

fn is_nullable(schema: &ObjectSchema) -> bool {
    schema.is_nullable().unwrap_or(false) || enum_contains_null(schema)
}

fn is_string_schema(schema: &ObjectSchema) -> bool {
    matches!(non_null_type(schema), Some(SchemaType::String))
}

/// Detects an OpenAPI 3.1 nullable enum spelled as `enum: [..., null]`
/// without `"null"` in the `type` set. Treated as nullable so the
/// containing field is wrapped in `Option<_>` while the enum itself
/// only carries its non-null variants.
fn enum_contains_null(schema: &ObjectSchema) -> bool {
    schema.enum_values.iter().any(serde_json::Value::is_null)
}

fn primitive_type(ty: &SchemaType) -> Option<syn::Type> {
    let out: syn::Type = match ty {
        SchemaType::String => parse_quote!(String),
        SchemaType::Number => parse_quote!(f64),
        SchemaType::Boolean => parse_quote!(bool),
        SchemaType::Integer => parse_quote!(i64),
        SchemaType::Null => parse_quote!(()),
        SchemaType::Array | SchemaType::Object => return None,
    };
    Some(out)
}

impl<'a> crate::Generator<'a> {
    /// Applies `format` overrides for both numeric and string primitives.
    ///
    /// Numeric overrides (`int32`, `uint64`, `float`, ...) are always
    /// applied. String overrides (`date-time`, `uuid`, `byte`,
    /// `binary`) only kick in when the matching flag is set in
    /// [`crate::BuildOptions`]; otherwise the base type (usually
    /// `String`) is kept.
    pub(crate) fn apply_format(&self, ty: syn::Type, schema: &ObjectSchema) -> syn::Type {
        let Some(format) = schema.format.as_deref() else {
            return ty;
        };
        let overridden: Option<syn::Type> = match format {
            // Numeric formats — emitted unconditionally.
            "int32" => Some(parse_quote!(i32)),
            "int64" => Some(parse_quote!(i64)),
            "uint32" => Some(parse_quote!(u32)),
            "uint64" => Some(parse_quote!(u64)),
            "float" => Some(parse_quote!(f32)),
            "double" => Some(parse_quote!(f64)),

            // String formats — gated on the caller's BuildOptions so we
            // don't force downstream crates to add dependencies they
            // don't want.
            "date-time" if self.options.use_chrono => {
                Some(parse_quote!(::chrono::DateTime<::chrono::Utc>))
            }
            "date" if self.options.use_chrono => Some(parse_quote!(::chrono::NaiveDate)),
            "time" if self.options.use_chrono => Some(parse_quote!(::chrono::NaiveTime)),
            "uuid" if self.options.use_uuid => Some(parse_quote!(::uuid::Uuid)),
            "byte" if self.options.use_base64_string => {
                let path = crate::constants::runtime_path("Base64String");
                Some(parse_quote!(#path))
            }
            // `binary` payloads are the raw request/response body shape
            // (multipart / octet-stream). `bytes::Bytes` is always
            // available through the runtime, so no opt-in
            // needed.
            "binary" => Some(parse_quote!(::bytes::Bytes)),

            _ => None,
        };
        overridden.unwrap_or(ty)
    }
}

fn maybe_optionalise(ty: syn::Type, nullable: bool) -> syn::Type {
    if nullable {
        parse_quote!(Option<#ty>)
    } else {
        ty
    }
}

/// Builds a `pub enum` for a string schema with `enum` values. Produces
/// both the enum item and a matching [`Display`] impl so the variant
/// renders as its wire value — parameter encoding in the generated
/// `MakeRequest` impl stringifies values through `ToString`.
///
/// Returns a vec of items: the enum first, followed by the Display impl.
fn build_string_enum_items(
    type_name: &syn::Ident,
    schema: &ObjectSchema,
    attrs: Vec<syn::Attribute>,
) -> Result<Vec<syn::Item>, Error> {
    let mut variants: Vec<syn::Variant> = Vec::with_capacity(schema.enum_values.len());
    let mut display_arms: Vec<proc_macro2::TokenStream> =
        Vec::with_capacity(schema.enum_values.len());
    for value in &schema.enum_values {
        // OpenAPI 3.1 spells nullable enums as `enum: [..., null]`. The
        // null is consumed by `enum_contains_null` to mark the field
        // `Option<_>`; the enum itself only carries the non-null
        // variants, so skip it here.
        if value.is_null() {
            continue;
        }
        let Some(raw) = value.as_str() else {
            return Err(Error::Unsupported(format!(
                "non-string enum variant in string schema: {value}"
            )));
        };
        let variant_ident = make_ident(&crate::naming::to_pascal_case(raw));
        let rename = if variant_ident == raw {
            quote! {}
        } else {
            quote! { #[serde(rename = #raw)] }
        };
        variants.push(parse_quote! {
            #rename
            #variant_ident
        });
        display_arms.push(quote! {
            Self::#variant_ident => ::std::write!(__f, #raw)
        });
    }

    let enum_item: syn::Item = parse_quote! {
        #(#attrs)*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
        pub enum #type_name {
            #(#variants,)*
        }
    };
    let display_impl: syn::Item = parse_quote! {
        impl ::std::fmt::Display for #type_name {
            fn fmt(&self, __f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                match self {
                    #(#display_arms,)*
                }
            }
        }
    };
    Ok(vec![enum_item, display_impl])
}

/// Doc-level attrs (title, description, examples, deprecated) without any
/// derives. Used by enum-building which supplies its own derive list.
fn doc_attrs(schema: &ObjectSchema) -> Vec<syn::Attribute> {
    let mut attrs: Vec<syn::Attribute> = Vec::new();
    if let Some(dep) = deprecated_attr(schema.deprecated) {
        attrs.push(dep);
    }
    push_schema_docs(
        &mut attrs,
        schema.title.as_deref(),
        schema.description.as_deref(),
        &schema.examples,
    );
    attrs
}

/// Full attrs for a struct/type-alias: derives + doc attrs.
fn struct_attrs(schema: &ObjectSchema) -> Vec<syn::Attribute> {
    let mut attrs: Vec<syn::Attribute> = Vec::new();
    attrs.push(parse_quote! {
        #[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
    });
    attrs.extend(doc_attrs(schema));
    attrs
}

fn serde_rename_attr(raw: &str, ident: &syn::Ident) -> Option<syn::Attribute> {
    let serialized = ident.to_string();
    let canonical = serialized.strip_prefix("r#").unwrap_or(&serialized);
    if canonical == raw {
        return None;
    }
    Some(parse_quote!(#[serde(rename = #raw)]))
}

fn json_any() -> syn::Type {
    parse_quote!(::serde_json::Value)
}

/// If the schema is `allOf: [$ref]` (optionally with `nullable: true` in
/// another slot), return the single reference. Used to thread through
/// `allOf` wrappers that are typically introduced just to attach nullability.
fn allof_single_ref(schema: &ObjectSchema) -> Option<&Schema> {
    if schema.all_of.len() != 1 {
        return None;
    }
    if !schema.properties.is_empty()
        || schema.schema_type.is_some()
        || !schema.any_of.is_empty()
        || !schema.one_of.is_empty()
    {
        return None;
    }
    schema.all_of.first()
}

// ---------------------------------------------------------------------------
// `allOf` flattening.
//
// `allOf` in OpenAPI means "this schema is the conjunction of every member",
// which for object-like members is equivalent to merging their properties.
// We resolve every member (including refs) and fold their properties,
// required lists, and `additionalProperties` into a single synthesised
// [`ObjectSchema`], then hand the result to the standard struct path.
//
// Members that are not object-like (primitives, enums, oneOf/anyOf) cause
// the merge to be skipped entirely — the caller then falls back to its
// existing behaviour, which generally means a `Fallback` type alias. This
// keeps the merge logic predictable instead of trying to express polymorphic
// conjunctions in Rust's type system.
// ---------------------------------------------------------------------------

/// Returns a merged [`ObjectSchema`] when `schema` has an `allOf` clause
/// whose members are all object-like, otherwise `None`.
fn flatten_all_of(schema: &ObjectSchema, spec: &oas3::Spec) -> Option<ObjectSchema> {
    if schema.all_of.is_empty() {
        return None;
    }
    if !schema.one_of.is_empty() || !schema.any_of.is_empty() {
        return None;
    }

    let mut merged = schema_without_all_of(schema);
    for member in &schema.all_of {
        let resolved = resolve_object_member(member, spec)?;
        if !is_object_like_for_merge(&resolved) {
            return None;
        }
        fold_into(&mut merged, &resolved, spec)?;
    }
    Some(merged)
}

/// Resolves one `Schema` member of a logical clause (`allOf` / `oneOf` /
/// `anyOf`) into an [`ObjectSchema`]. Boolean members are not
/// representable as object-shaped schemas, so they signal "skip this
/// merge" by returning `None` — same outcome as a primitive member.
fn resolve_object_member(member: &Schema, spec: &oas3::Spec) -> Option<ObjectSchema> {
    match member {
        Schema::Boolean(_) => None,
        Schema::Object(boxed) => match boxed.as_ref() {
            ObjectOrReference::Object(inner) => Some(inner.clone()),
            ObjectOrReference::Ref { ref_path, .. } => {
                let resolved = <Schema as oas3::spec::FromRef>::from_ref(spec, ref_path).ok()?;
                Some(schema_to_object(&resolved))
            }
        },
    }
}

/// Clones `schema` into the seed for a merge, stripping the `allOf` so the
/// caller can re-process the merged result through the normal path without
/// recursing back into this function.
fn schema_without_all_of(schema: &ObjectSchema) -> ObjectSchema {
    let mut seed = schema.clone();
    seed.all_of.clear();
    seed
}

/// A member schema is mergeable if it's a plain object-shaped schema — has
/// properties, `additionalProperties`, or its own `allOf` — without any
/// primitive `type`, `enum`, or sum-type clauses that would conflict with
/// struct generation.
fn is_object_like_for_merge(schema: &ObjectSchema) -> bool {
    let has_incompatible_type = match &schema.schema_type {
        Some(SchemaTypeSet::Single(SchemaType::Object)) => false,
        Some(SchemaTypeSet::Single(_)) => true,
        Some(SchemaTypeSet::Multiple(types)) => types
            .iter()
            .any(|t| !matches!(t, SchemaType::Object | SchemaType::Null)),
        None => false,
    };
    if has_incompatible_type {
        return false;
    }
    if !schema.enum_values.is_empty()
        || schema.const_value.is_some()
        || !schema.one_of.is_empty()
        || !schema.any_of.is_empty()
    {
        return false;
    }
    true
}

/// Folds `member`'s fields into `target`. Recurses through nested `allOf`
/// so a deep chain collapses to one struct.
fn fold_into(target: &mut ObjectSchema, member: &ObjectSchema, spec: &oas3::Spec) -> Option<()> {
    for nested in &member.all_of {
        let resolved = resolve_object_member(nested, spec)?;
        if !is_object_like_for_merge(&resolved) {
            return None;
        }
        fold_into(target, &resolved, spec)?;
    }

    // `oas3::Map` is order-preserving but lacks an `entry` API, so guard
    // each insert with `contains_key` to keep the "first-write wins"
    // semantics that the original `BTreeMap::entry().or_insert_with`
    // expressed.
    for (name, prop) in &member.properties {
        if !target.properties.contains_key(name) {
            target.properties.insert(name.clone(), prop.clone());
        }
    }
    for req in &member.required {
        if !target.required.contains(req) {
            target.required.push(req.clone());
        }
    }
    if target.additional_properties.is_none() {
        target.additional_properties = member.additional_properties.clone();
    }
    if target.description.is_none() {
        target.description = member.description.clone();
    }
    if target.title.is_none() {
        target.title = member.title.clone();
    }
    if target.schema_type.is_none() {
        target.schema_type = member.schema_type.clone();
    }
    Some(())
}

// ---------------------------------------------------------------------------
// `oneOf` / `anyOf` sum-enum generation.
// ---------------------------------------------------------------------------

/// Describes how a sum enum should be tagged on the wire.
#[derive(Debug, Clone)]
enum SumKind {
    /// `oneOf` without a discriminator, or `anyOf` — serialises with
    /// `#[serde(untagged)]`.
    Untagged,
    /// `oneOf` with an OpenAPI discriminator — serialises with
    /// `#[serde(tag = "...")]` (internally tagged).
    InternallyTagged(Discriminator),
}

/// Detects whether a schema should be generated as a sum enum. Returns the
/// tagging strategy if so.
///
/// `oneOf` takes precedence over `anyOf` when both are present, mirroring
/// their relative strictness. A schema that also declares `type`, `properties`
/// or `allOf` is not treated as a pure sum type; those are out of scope for
/// this path.
fn detect_sum_kind(schema: &ObjectSchema) -> Option<SumKind> {
    if !schema.properties.is_empty() || !schema.all_of.is_empty() {
        return None;
    }
    if !schema.one_of.is_empty() {
        return Some(match schema.discriminator.as_ref() {
            Some(d) => SumKind::InternallyTagged(d.clone()),
            None => SumKind::Untagged,
        });
    }
    if !schema.any_of.is_empty() {
        return Some(SumKind::Untagged);
    }
    None
}

/// Returns the member list backing a sum schema. Callers should only invoke
/// this when [`detect_sum_kind`] reported a match.
fn sum_members(schema: &ObjectSchema) -> &[Schema] {
    if !schema.one_of.is_empty() {
        &schema.one_of
    } else {
        &schema.any_of
    }
}

/// A resolved sum-enum variant ready for tokenisation.
struct SumVariant {
    ident: syn::Ident,
    inner_type: syn::Type,
    /// Override for serde's variant name: used for discriminator mappings
    /// and any time the identifier doesn't round-trip to the wire name.
    serde_rename: Option<String>,
}

/// Attributes (derives + docs + serde tagging) for the emitted sum enum.
fn sum_enum_attrs(schema: &ObjectSchema, kind: &SumKind) -> Vec<syn::Attribute> {
    let mut attrs: Vec<syn::Attribute> = Vec::new();
    attrs.push(parse_quote! {
        #[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
    });
    match kind {
        SumKind::Untagged => {
            attrs.push(parse_quote!(#[serde(untagged)]));
        }
        SumKind::InternallyTagged(d) => {
            let tag = d.property_name.as_str();
            attrs.push(parse_quote!(#[serde(tag = #tag)]));
        }
    }
    attrs.extend(doc_attrs(schema));
    attrs
}

/// Picks a unique variant identifier and the `#[serde(rename = "...")]` value
/// (if any) for one member.
///
/// For internally-tagged enums, the rename is taken from the discriminator
/// mapping when present; for ref-backed variants it falls back to the ref'd
/// component name. For untagged enums the rename only fires when the ident
/// had to be mangled away from the source name.
fn sum_variant_ident(
    parent_type: &syn::Ident,
    index: usize,
    kind: &SumKind,
    ref_name: Option<&str>,
    used_names: &mut Vec<String>,
) -> (syn::Ident, Option<String>) {
    let (preferred, wire_name) = match (kind, ref_name) {
        (SumKind::InternallyTagged(d), Some(name)) => {
            let mapped = discriminator_key_for(d, name);
            (mapped.clone().unwrap_or_else(|| name.to_owned()), mapped)
        }
        (SumKind::InternallyTagged(_), None) => {
            let fallback = format!("{parent_type}Variant{index}");
            (fallback, None)
        }
        (SumKind::Untagged, Some(name)) => (name.to_owned(), None),
        (SumKind::Untagged, None) => {
            let fallback = format!("{parent_type}Variant{index}");
            (fallback, None)
        }
    };

    let ident = make_ident(&crate::naming::to_pascal_case(&preferred));
    let unique = disambiguate(ident.clone(), used_names);
    used_names.push(unique.to_string());

    let canonical = ident.to_string();
    let canonical = canonical.strip_prefix("r#").unwrap_or(&canonical);
    let rename = match (wire_name, canonical == preferred) {
        (Some(name), _) => Some(name),
        (None, false) => Some(preferred),
        (None, true) => None,
    };
    (unique, rename)
}

/// Returns the mapping key that points at `component_name`, if the
/// discriminator mapping lists one. OpenAPI discriminators map wire values
/// to component names, so this reverse lookup determines the `#[serde(rename)]`
/// value for the variant.
fn discriminator_key_for(d: &Discriminator, component_name: &str) -> Option<String> {
    let mapping = d.mapping.as_ref()?;
    mapping.iter().find_map(|(key, target)| {
        let stripped = target.strip_prefix(SCHEMA_REF_PREFIX).unwrap_or(target);
        (stripped == component_name).then(|| key.clone())
    })
}

/// Appends a numeric suffix until the ident doesn't collide with a prior
/// variant in the same enum.
fn disambiguate(candidate: syn::Ident, used: &[String]) -> syn::Ident {
    let base = candidate.to_string();
    if !used.contains(&base) {
        return candidate;
    }
    let mut counter = 2usize;
    loop {
        let next = format!("{base}{counter}");
        if !used.contains(&next) {
            return syn::Ident::new(&next, proc_macro2::Span::call_site());
        }
        counter += 1;
    }
}

/// Inner types we know unconditionally implement [`std::fmt::Display`].
/// Used by [`sum_display_impl`] to decide whether emitting a `Display`
/// impl on a sum enum is safe — emitting one over a non-`Display` inner
/// would fail to compile, so we only opt in when every variant is a
/// known-displayable primitive.
///
/// The list mirrors the primitives produced by [`primitive_type`] plus
/// the formats from [`apply_format`]; widening it requires confirming
/// the new type implements `Display` in `core`/`std`.
const DISPLAY_INNER_TYPES: &[&str] = &["String", "bool", "i32", "i64", "u32", "u64", "f32", "f64"];

/// Emits `Display` for a sum enum when every variant wraps a single
/// known-displayable primitive (see [`DISPLAY_INNER_TYPES`]).
///
/// Path parameters whose schema is `oneOf: [integer, string]` lower to
/// such enums; without a `Display` impl the generated path-rendering
/// code (which calls `ToString::to_string` on the field) fails to
/// compile. Returns `None` when at least one variant wraps a complex
/// type — a struct or another enum — because we can't statically verify
/// it implements `Display`.
fn sum_display_impl(enum_name: &syn::Ident, variants: &[SumVariant]) -> Option<syn::Item> {
    if variants.is_empty() {
        return None;
    }
    if !variants.iter().all(|v| is_display_inner(&v.inner_type)) {
        return None;
    }
    let arms: Vec<proc_macro2::TokenStream> = variants
        .iter()
        .map(|v| {
            let ident = &v.ident;
            quote! {
                #enum_name::#ident(__inner) => ::std::fmt::Display::fmt(__inner, f),
            }
        })
        .collect();
    Some(parse_quote! {
        impl ::std::fmt::Display for #enum_name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                match self {
                    #(#arms)*
                }
            }
        }
    })
}

fn is_display_inner(ty: &syn::Type) -> bool {
    let syn::Type::Path(path) = ty else {
        return false;
    };
    let Some(ident) = path.path.get_ident() else {
        return false;
    };
    DISPLAY_INNER_TYPES.contains(&ident.to_string().as_str())
}

/// Emits `From<Inner> for Enum` and `TryFrom<Enum> for Inner` impls for one
/// variant. Returns an empty list for variants whose inner type would make
/// the impls useless or invalid (currently none; the dedup at the caller
/// handles duplicate inner types).
fn sum_conversion_impls(enum_name: &syn::Ident, variant: &SumVariant) -> Vec<syn::Item> {
    let variant_ident = &variant.ident;
    let inner_ty = &variant.inner_type;
    let from_impl: syn::Item = parse_quote! {
        impl ::std::convert::From<#inner_ty> for #enum_name {
            fn from(value: #inner_ty) -> Self {
                #enum_name::#variant_ident(value)
            }
        }
    };
    let try_from_impl: syn::Item = parse_quote! {
        impl ::std::convert::TryFrom<#enum_name> for #inner_ty {
            type Error = #enum_name;

            /// # Errors
            ///
            /// Returns the original enum value if it does not match the
            /// `#variant_ident` variant.
            fn try_from(value: #enum_name) -> ::std::result::Result<Self, Self::Error> {
                match value {
                    #enum_name::#variant_ident(inner) => ::std::result::Result::Ok(inner),
                    other => ::std::result::Result::Err(other),
                }
            }
        }
    };
    vec![from_impl, try_from_impl]
}

/// Doc/attribute bits extracted from an inline field schema, separate from
/// the concrete Rust type so the caller can decide whether to wrap in
/// `Option` first.
#[derive(Default)]
pub(crate) struct InlineDocs {
    pub(crate) description: Option<String>,
    pub(crate) deprecated: Option<Vec<syn::Attribute>>,
}

impl InlineDocs {
    fn from_schema(schema: &ObjectSchema) -> Self {
        let deprecated = deprecated_attr(schema.deprecated).map(|a| vec![a]);
        Self {
            description: schema.description.clone(),
            deprecated,
        }
    }

    /// Produces a doc note for a `$ref` that could not be resolved to a
    /// generated type, so the user can see which pointer fell through.
    fn unresolvable_ref(ref_path: &str) -> Self {
        Self {
            description: Some(format!(
                "Unresolved `$ref`: `{ref_path}`. The source spec uses a \
                 reference shape (e.g. JSON Schema `$id`) that this \
                 generator cannot map to a typed component; the field falls \
                 back to `serde_json::Value`."
            )),
            deprecated: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Recursion handling.
//
// A struct field that transitively references its own type would produce
// an infinite-size Rust type. We rewrite any such field to hold the value
// through a `Box` so the recursive definitions compile.
//
// Fields whose types are already indirected (`Vec<T>`, `HashMap<_, T>`,
// `Box<T>`, `Option<Vec<T>>`, ...) do not need additional boxing; the heap
// indirection provided by those containers already breaks the size cycle.
// ---------------------------------------------------------------------------

/// Wraps recursive field references in `Box` across all generated items.
pub fn box_recursive_cycles(items: &mut [syn::Item]) {
    let struct_idents: std::collections::BTreeSet<String> = items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(s) => Some(s.ident.to_string()),
            _ => None,
        })
        .collect();

    let edges = collect_struct_edges(items, &struct_idents);
    let reachable = compute_reachability(&edges);

    for item in items.iter_mut() {
        let syn::Item::Struct(structure) = item else {
            continue;
        };
        let owner = structure.ident.to_string();
        for field in structure.fields.iter_mut() {
            box_field_if_recursive(field, &owner, &reachable, &struct_idents);
        }
    }
}

/// Adjacency: `struct_ident → set of struct idents it directly references`.
fn collect_struct_edges(
    items: &[syn::Item],
    struct_idents: &std::collections::BTreeSet<String>,
) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
    let mut edges: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for item in items {
        let syn::Item::Struct(structure) = item else {
            continue;
        };
        let from = structure.ident.to_string();
        let entry = edges.entry(from).or_default();
        for field in structure.fields.iter() {
            collect_referenced_idents(&field.ty, struct_idents, entry);
        }
    }
    edges
}

/// Adds any struct-level type idents referenced by `ty` into `out`.
///
/// Walks through the transparent wrappers (`Option`, `Box`, `Vec`,
/// `HashMap`, `BTreeMap`) so `Option<Vec<Node>>` counts `Node` as a
/// referenced ident.
fn collect_referenced_idents(
    ty: &syn::Type,
    struct_idents: &std::collections::BTreeSet<String>,
    out: &mut std::collections::BTreeSet<String>,
) {
    let syn::Type::Path(path) = ty else {
        return;
    };
    let Some(last) = path.path.segments.last() else {
        return;
    };
    let last_name = last.ident.to_string();

    match &last.arguments {
        syn::PathArguments::None => {
            if struct_idents.contains(&last_name) {
                out.insert(last_name);
            }
        }
        syn::PathArguments::AngleBracketed(args) => {
            for arg in &args.args {
                if let syn::GenericArgument::Type(inner) = arg {
                    collect_referenced_idents(inner, struct_idents, out);
                }
            }
        }
        syn::PathArguments::Parenthesized(_) => {}
    }
}

/// For every struct ident, the set of struct idents transitively reachable
/// through field references.
fn compute_reachability(
    edges: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
    let mut reachable: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for source in edges.keys() {
        let mut visited = std::collections::BTreeSet::new();
        let mut stack: Vec<&str> = edges
            .get(source)
            .into_iter()
            .flat_map(|set| set.iter().map(String::as_str))
            .collect();
        while let Some(node) = stack.pop() {
            if !visited.insert(node.to_owned()) {
                continue;
            }
            if let Some(next) = edges.get(node) {
                stack.extend(next.iter().map(String::as_str));
            }
        }
        reachable.insert(source.clone(), visited);
    }
    reachable
}

/// Rewrites `field.ty` to `Box<_>` (respecting an outer `Option`) when it
/// directly references a struct that transitively reaches `owner`.
///
/// Fields that are already behind `Vec`, `HashMap`, `BTreeMap`, or `Box`
/// are not touched because those containers already break the size cycle.
fn box_field_if_recursive(
    field: &mut syn::Field,
    owner: &str,
    reachable: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    struct_idents: &std::collections::BTreeSet<String>,
) {
    let new_type = rewrite_type_for_cycle(&field.ty, owner, reachable, struct_idents);
    if let Some(new_type) = new_type {
        field.ty = new_type;
    }
}

/// Recursive helper that returns a rewritten type when boxing is needed.
fn rewrite_type_for_cycle(
    ty: &syn::Type,
    owner: &str,
    reachable: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    struct_idents: &std::collections::BTreeSet<String>,
) -> Option<syn::Type> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    let last = path.path.segments.last()?;
    let last_name = last.ident.to_string();

    match &last.arguments {
        syn::PathArguments::None => {
            if struct_idents.contains(&last_name) && reaches_owner(&last_name, owner, reachable) {
                let original = ty.clone();
                return Some(parse_quote!(Box<#original>));
            }
            None
        }
        syn::PathArguments::AngleBracketed(_) => {
            // `Option<T>` is the only transparent wrapper we unwrap — other
            // container types (`Vec`, `HashMap`, `BTreeMap`, `Box`) already
            // provide the heap indirection that breaks the size cycle.
            if last_name != "Option" {
                return None;
            }
            let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
                return None;
            };
            let inner = args.args.iter().find_map(|a| match a {
                syn::GenericArgument::Type(t) => Some(t),
                _ => None,
            })?;
            let rewritten = rewrite_type_for_cycle(inner, owner, reachable, struct_idents)?;
            Some(parse_quote!(Option<#rewritten>))
        }
        syn::PathArguments::Parenthesized(_) => None,
    }
}

/// Returns `true` if `candidate` can transitively reach `owner` (including
/// being `owner` itself, which is the direct self-reference case).
fn reaches_owner(
    candidate: &str,
    owner: &str,
    reachable: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
) -> bool {
    if candidate == owner {
        return true;
    }
    reachable
        .get(candidate)
        .is_some_and(|set| set.contains(owner))
}
