//! Per-operation code generation: request struct, response enum, and
//! inherent metadata impl.

use std::collections::BTreeMap;

use http::Method;
use oas3::spec::{
    MediaType, ObjectOrReference, ObjectSchema, Operation, Parameter, ParameterIn, PathItem,
};
use quote::quote;
use syn::parse_quote;

use crate::{
    Error, Generator,
    docs::{deprecated_attr, push_field_docs, push_schema_docs},
    naming::{field_ident, make_ident, to_pascal_case, type_ident},
};

/// Field name used for the request body in generated request structs.
const BODY_FIELD_NAME: &str = "body";

/// Variant name used for the `default` response branch.
const DEFAULT_RESPONSE_VARIANT: &str = "Default";

/// Headers that the OpenAPI spec says MUST be ignored when declared as
/// parameters. They're carried by the transport layer and mixing them into
/// the generated struct would be misleading.
const IGNORED_HEADER_NAMES: &[&str] = &["Accept", "Content-Type", "Authorization"];

impl<'a> Generator<'a> {
    /// Emits every operation declared on one path item.
    pub(crate) fn emit_path_item(&mut self, path: &str, item: &PathItem) -> Result<(), Error> {
        for (method, operation) in item.methods().into_iter() {
            self.emit_operation(path, &method, item, operation)?;
        }
        Ok(())
    }

    /// Generates the request struct, response enum, metadata impl, and
    /// runtime trait impls for one operation.
    fn emit_operation(
        &mut self,
        path: &str,
        method: &Method,
        path_item: &PathItem,
        operation: &Operation,
    ) -> Result<(), Error> {
        // Compute where this op's items land in the nested `operations`
        // module tree. All `store_*` calls until `clear_mod_path` will
        // target this module.
        let mod_path = crate::path_mod::mod_path(path, method);
        self.set_mod_path(mod_path.clone());

        // Path-mod layout: all types attached to one operation share a
        // module, so their Rust names collapse to fixed `Request` /
        // `Response`. The `mod_path` (e.g. `pets::by_id::get`) is what
        // disambiguates across operations.
        let request_ident = type_ident("Request");
        let response_ident = type_ident("Response");

        // Registry keys still need to be globally unique because
        // `items` / `type_paths` are flat maps. Use the full mod path
        // as a qualifier.
        let key_prefix = key_prefix_for(&mod_path);

        let param_slots = self.collect_parameters(&request_ident, path_item, operation)?;
        let body_slot = self.collect_request_body(&request_ident, operation)?;

        let request_item =
            build_request_struct(&request_ident, operation, &param_slots, body_slot.as_ref());
        self.store_named(
            format!("__op/{key_prefix}/Request"),
            request_ident.clone(),
            request_item,
        );

        let meta_item = build_metadata_impl(&request_ident, method, path, operation);
        self.store_unnamed(meta_item);

        let request_impl = build_make_request_impl(
            &request_ident,
            method,
            path,
            &param_slots,
            body_slot.as_ref(),
        );
        self.store_unnamed(request_impl);

        let response_variants = self.build_response_variants(&response_ident, operation)?;
        let response_item = build_response_enum(&response_ident, &response_variants);
        self.store_named(
            format!("__op/{key_prefix}/Response"),
            response_ident.clone(),
            response_item,
        );

        let response_impl = build_parse_response_impl(&response_ident, &response_variants);
        self.store_unnamed(response_impl);

        let operation_impl = build_operation_impl(&request_ident, &response_ident);
        self.store_unnamed(operation_impl);

        self.emit_op_level_servers(&request_ident, operation)?;

        self.clear_mod_path();
        Ok(())
    }

    /// Emits operation-scoped server types and the `with_server`
    /// inherent method when the operation declares its own `servers`.
    ///
    /// Types live next to the operation (in the same `operations`
    /// module) and are named `{Op}Server`, `{Op}ServerOption{i}`,
    /// plus nested variable-enum types following the same scheme as
    /// root-level servers.
    fn emit_op_level_servers(
        &mut self,
        request_ident: &syn::Ident,
        operation: &Operation,
    ) -> Result<(), Error> {
        if operation.servers.is_empty() {
            return Ok(());
        }

        // The op's servers live in the same `operations::<path>::<method>`
        // module as its `Request` / `Response`, so names don't need a
        // per-op prefix — `ServerOption0`, `ServerOption1`, `Server`
        // qualify via the mod path.
        let aggregate_ident = type_ident("Server");
        let key_prefix = key_prefix_for(&self.current_mod_path.clone());

        let mut option_idents: Vec<syn::Ident> = Vec::with_capacity(operation.servers.len());
        for (index, server) in operation.servers.iter().enumerate() {
            let option_ident = type_ident(&format!("ServerOption{index}"));
            self.emit_op_server_option(&key_prefix, &option_ident, server)?;
            option_idents.push(option_ident);
        }

        if option_idents.len() == 1 {
            let only = &option_idents[0];
            let alias: syn::Item = parse_quote! {
                pub type #aggregate_ident = #only;
            };
            self.store_named(
                format!("__op_server_agg/{key_prefix}/{aggregate_ident}"),
                aggregate_ident.clone(),
                alias,
            );
        } else {
            self.emit_op_server_aggregate(&key_prefix, &aggregate_ident, &option_idents);
        }

        // Inherent `with_server` method on the request type.
        let with_server_ty = crate::constants::runtime_path("WithServer");
        let with_server_method: syn::Item = parse_quote! {
            impl #request_ident {
                /// Routes this call against an operation-specific server.
                /// Only servers declared on this operation are accepted,
                /// so invalid combinations are caught at compile time.
                pub fn with_server(
                    self,
                    server: #aggregate_ident,
                ) -> #with_server_ty<Self> {
                    #with_server_ty::new(self, server)
                }
            }
        };
        self.store_unnamed(with_server_method);
        Ok(())
    }

    /// Thin wrapper that reuses the root-level server-option rendering
    /// path for op-level servers. `key_prefix` scopes the registry key
    /// so two ops with same `ServerOption0` name don't collide.
    fn emit_op_server_option(
        &mut self,
        key_prefix: &str,
        option_ident: &syn::Ident,
        server: &oas3::spec::Server,
    ) -> Result<(), Error> {
        crate::servers::emit_server_option_in_stage(self, key_prefix, option_ident, server)
    }

    /// Emits the per-op aggregate enum + `Server` impl + `Default`.
    fn emit_op_server_aggregate(
        &mut self,
        key_prefix: &str,
        aggregate_ident: &syn::Ident,
        option_idents: &[syn::Ident],
    ) {
        crate::servers::emit_aggregate_in_stage(self, key_prefix, aggregate_ident, option_idents);
    }

    /// Resolves and merges path-level and operation-level parameters,
    /// returning [`ParamSlot`]s with the final Rust field ident and type
    /// precomputed — so every downstream consumer (struct builder,
    /// request impl builder) sees the same names.
    fn collect_parameters(
        &mut self,
        parent: &syn::Ident,
        path_item: &PathItem,
        operation: &Operation,
    ) -> Result<Vec<ParamSlot>, Error> {
        let mut merged: Vec<Parameter> = Vec::new();

        for param_or_ref in &path_item.parameters {
            let resolved = param_or_ref.resolve(self.spec)?;
            upsert_parameter(&mut merged, resolved);
        }
        for param_or_ref in &operation.parameters {
            let resolved = param_or_ref.resolve(self.spec)?;
            upsert_parameter(&mut merged, resolved);
        }

        let filtered: Vec<Parameter> = merged
            .into_iter()
            .filter(|p| !should_ignore_parameter(p))
            .collect();

        let mut used_field_names: BTreeMap<String, usize> = BTreeMap::new();
        let mut slots: Vec<ParamSlot> = Vec::with_capacity(filtered.len());
        for parameter in filtered {
            let base_ident = field_ident(&parameter.name);
            let field_ident =
                disambiguate_field(&base_ident, parameter.location, &mut used_field_names);

            let inner_ty = match parameter.schema.as_ref() {
                Some(schema_or_ref) => {
                    self.field_type_from_schema(parent, &parameter.name, schema_or_ref)?
                }
                // Parameters declared via `content` fall back to opaque JSON.
                None => parse_quote!(::serde_json::Value),
            };

            slots.push(ParamSlot {
                field_ident,
                parameter,
                inner_ty,
            });
        }
        Ok(slots)
    }

    /// Resolves the operation's request body (if any) into a [`BodySlot`]
    /// that carries everything downstream builders need (field ident,
    /// type, and required flag).
    fn collect_request_body(
        &mut self,
        parent: &syn::Ident,
        operation: &Operation,
    ) -> Result<Option<BodySlot>, Error> {
        let Some(body_or_ref) = operation.request_body.as_ref() else {
            return Ok(None);
        };
        let body = body_or_ref.resolve(self.spec)?;
        let Some((_, media)) = preferred_media_type(&body.content) else {
            return Ok(None);
        };
        let Some(schema_or_ref) = media.schema.as_ref() else {
            return Ok(None);
        };

        let inner_ty = self.field_type_from_schema(parent, BODY_FIELD_NAME, schema_or_ref)?;
        Ok(Some(BodySlot {
            ident: make_ident(BODY_FIELD_NAME),
            inner_ty,
            description: body.description.clone(),
            required: body.required.unwrap_or(false),
        }))
    }

    /// Resolves the response payload types for every declared status code.
    fn build_response_variants(
        &mut self,
        enum_ident: &syn::Ident,
        operation: &Operation,
    ) -> Result<Vec<ResponseVariant>, Error> {
        let mut out: Vec<ResponseVariant> = Vec::new();
        let Some(map) = operation.responses.as_ref() else {
            return Ok(out);
        };
        for (status, resp_or_ref) in map {
            let Ok(response) = resp_or_ref.resolve(self.spec) else {
                continue;
            };
            let variant_ident = response_variant_ident(status);
            let inner_type = match preferred_media_type(&response.content) {
                Some((_, media)) => match media.schema.as_ref() {
                    Some(schema_or_ref) => Some(self.field_type_from_schema(
                        enum_ident,
                        &format!("{variant_ident}Body"),
                        schema_or_ref,
                    )?),
                    None => None,
                },
                None => None,
            };
            out.push(ResponseVariant {
                status: status.clone(),
                variant_ident,
                inner_type,
                description: response.description.clone(),
            });
        }
        Ok(out)
    }

    /// Resolves a schema reference/inline schema into the Rust type we'd
    /// put in a field. Delegates to the schema stage's inline-type logic
    /// so component `$ref`s hit the shared registry.
    fn field_type_from_schema(
        &mut self,
        parent: &syn::Ident,
        hint: &str,
        schema_or_ref: &ObjectOrReference<ObjectSchema>,
    ) -> Result<syn::Type, Error> {
        let (ty, _) = self.inline_type(parent, hint, schema_or_ref)?;
        Ok(ty)
    }
}

/// Fully-resolved description of one request parameter, containing both
/// the OpenAPI source and the Rust field it projects into.
struct ParamSlot {
    /// Final Rust field ident (post-snake_case, post-disambiguation).
    field_ident: syn::Ident,
    /// Underlying spec parameter.
    parameter: Parameter,
    /// Rust type *before* `Option<...>` wrapping. The wrapping is added
    /// at struct-build time when the parameter is optional.
    inner_ty: syn::Type,
}

impl ParamSlot {
    fn is_required(&self) -> bool {
        parameter_is_required(&self.parameter)
    }

    fn struct_field_type(&self) -> syn::Type {
        let inner = &self.inner_ty;
        if self.is_required() {
            inner.clone()
        } else {
            parse_quote!(Option<#inner>)
        }
    }
}

/// Fully-resolved description of an optional request body.
struct BodySlot {
    ident: syn::Ident,
    inner_ty: syn::Type,
    description: Option<String>,
    required: bool,
}

impl BodySlot {
    fn struct_field_type(&self) -> syn::Type {
        let inner = &self.inner_ty;
        if self.required {
            inner.clone()
        } else {
            parse_quote!(Option<#inner>)
        }
    }
}

/// Fully-resolved description of one response variant.
struct ResponseVariant {
    /// Raw OpenAPI status key (`"200"`, `"default"`, `"2XX"`, ...).
    status: String,
    /// Rust variant ident (`Status200`, `Default`, `Status2XX`, ...).
    variant_ident: syn::Ident,
    /// Payload type, `None` for unit variants.
    inner_type: Option<syn::Type>,
    /// Free-form description lifted from the spec.
    description: Option<String>,
}

/// Inserts or replaces a parameter in the merged list using the spec's
/// identity rule `(name, location)`. Later writes win, matching the
/// operation-overrides-path semantics of OpenAPI.
fn upsert_parameter(into: &mut Vec<Parameter>, incoming: Parameter) {
    if let Some(slot) = into
        .iter_mut()
        .find(|p| p.name == incoming.name && p.location == incoming.location)
    {
        *slot = incoming;
        return;
    }
    into.push(incoming);
}

/// Tail-facing parameter data used to construct one struct field.
fn parameter_is_required(parameter: &Parameter) -> bool {
    match parameter.location {
        ParameterIn::Path => true,
        _ => parameter.required.unwrap_or(false),
    }
}

fn parameter_description(parameter: &Parameter) -> Option<String> {
    parameter.description.clone()
}

/// Headers that must be ignored, and cookies that we skip for now.
fn should_ignore_parameter(parameter: &Parameter) -> bool {
    if parameter.location == ParameterIn::Cookie {
        return true;
    }
    if parameter.location == ParameterIn::Header
        && IGNORED_HEADER_NAMES
            .iter()
            .any(|banned| banned.eq_ignore_ascii_case(&parameter.name))
    {
        return true;
    }
    false
}

/// Returns the first media type in `content` whose key is JSON-shaped
/// (`application/json`, `application/problem+json`, ...), falling back to
/// the first entry if none look like JSON.
fn preferred_media_type(content: &BTreeMap<String, MediaType>) -> Option<(&String, &MediaType)> {
    if let Some(json) = content
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("application/json"))
    {
        return Some(json);
    }
    if let Some(json_like) = content
        .iter()
        .find(|(k, _)| k.to_ascii_lowercase().ends_with("+json"))
    {
        return Some(json_like);
    }
    content.iter().next()
}

/// Synthesises a PascalCase operation name. Prefers `operationId`; falls
/// back to `{Method}{Path}` when absent. Used by the upcoming
/// `ClientExt` convenience layer (method names), so kept despite
/// currently being unreferenced from the path-mod rewrite.
#[allow(dead_code)]
fn operation_name(method: &Method, path: &str, operation: &Operation) -> String {
    if let Some(id) = operation.operation_id.as_deref() {
        return to_pascal_case(id);
    }
    let mut raw = String::new();
    raw.push_str(method.as_str());
    raw.push(' ');
    raw.push_str(path);
    to_pascal_case(&raw)
}

/// Joins a mod path into a registry-key qualifier. `["pets", "by_id", "get"]`
/// → `"pets/by_id/get"`. Used by `store_named` / `store_unnamed` callers
/// to keep keys unique across ops that now share type names like
/// `Request` / `Response` / `Server`.
fn key_prefix_for(mod_path: &[String]) -> String {
    mod_path.join("/")
}

/// Produces a deduplicated field ident: when two parameters normalise to
/// the same snake_case ident (e.g. path `id` and query `id`), the later
/// ones get a `_path` / `_query` / `_header` suffix. The serde rename
/// already carries the original wire name, so this only affects Rust-side
/// access.
fn disambiguate_field(
    base: &syn::Ident,
    location: ParameterIn,
    used: &mut BTreeMap<String, usize>,
) -> syn::Ident {
    let base_str = base.to_string();
    let count = used.entry(base_str.clone()).or_insert(0);
    *count += 1;
    if *count == 1 {
        return base.clone();
    }
    let suffix = match location {
        ParameterIn::Path => "path",
        ParameterIn::Query => "query",
        ParameterIn::Header => "header",
        ParameterIn::Cookie => "cookie",
    };
    let stripped = base_str.strip_prefix("r#").unwrap_or(&base_str);
    make_ident(&format!("{stripped}_{suffix}"))
}

/// Turns a [`ParamSlot`] into its struct field representation.
///
/// The request struct is never serialised through `serde` — the wire
/// name is applied at request-building time inside `MakeRequest` —
/// so no `#[serde(rename)]` attribute is emitted here.
fn param_to_field(slot: &ParamSlot) -> syn::Field {
    let mut attrs: Vec<syn::Attribute> = Vec::new();
    push_field_docs(
        &mut attrs,
        parameter_description(&slot.parameter).as_deref(),
    );
    if let Some(dep) = deprecated_attr(slot.parameter.deprecated) {
        attrs.push(dep);
    }

    syn::Field {
        attrs,
        vis: parse_quote!(pub),
        mutability: syn::FieldMutability::None,
        ident: Some(slot.field_ident.clone()),
        colon_token: Some(Default::default()),
        ty: slot.struct_field_type(),
    }
}

/// Turns a [`BodySlot`] into its struct field representation.
fn body_to_field(slot: &BodySlot) -> syn::Field {
    let mut attrs: Vec<syn::Attribute> = Vec::new();
    push_field_docs(&mut attrs, slot.description.as_deref());

    syn::Field {
        attrs,
        vis: parse_quote!(pub),
        mutability: syn::FieldMutability::None,
        ident: Some(slot.ident.clone()),
        colon_token: Some(Default::default()),
        ty: slot.struct_field_type(),
    }
}

/// Assembles the request struct from resolved parameter and body slots.
fn build_request_struct(
    ident: &syn::Ident,
    operation: &Operation,
    params: &[ParamSlot],
    body: Option<&BodySlot>,
) -> syn::Item {
    let mut fields: Vec<syn::Field> = params.iter().map(param_to_field).collect();
    if let Some(body) = body {
        fields.push(body_to_field(body));
    }

    let mut attrs: Vec<syn::Attribute> = Vec::new();
    attrs.push(parse_quote! {
        #[derive(Debug, Clone, PartialEq)]
    });
    if let Some(dep) = deprecated_attr(operation.deprecated) {
        attrs.push(dep);
    }
    push_schema_docs(
        &mut attrs,
        operation.summary.as_deref(),
        operation.description.as_deref(),
        &[],
    );

    parse_quote! {
        #(#attrs)*
        pub struct #ident {
            #(#fields,)*
        }
    }
}

/// Assembles the response enum from resolved variants.
fn build_response_enum(ident: &syn::Ident, variants: &[ResponseVariant]) -> syn::Item {
    if variants.is_empty() {
        return parse_quote! {
            #[derive(Debug, Clone, PartialEq)]
            pub enum #ident {
                /// No responses declared by the spec; use this branch
                /// to represent a successful unit response.
                Empty,
            }
        };
    }

    let variant_tokens: Vec<proc_macro2::TokenStream> = variants
        .iter()
        .map(|v| {
            let variant_ident = &v.variant_ident;
            let doc = v.description.as_deref().map(|d| quote! { #[doc = #d] });
            match &v.inner_type {
                Some(ty) => quote! {
                    #doc
                    #variant_ident(#ty)
                },
                None => quote! {
                    #doc
                    #variant_ident
                },
            }
        })
        .collect();

    parse_quote! {
        #[derive(Debug, Clone, PartialEq)]
        pub enum #ident {
            #(#variant_tokens,)*
        }
    }
}

/// Builds the `impl Request { const METHOD, const PATH_TEMPLATE }` block.
fn build_metadata_impl(
    request_ident: &syn::Ident,
    method: &Method,
    path: &str,
    operation: &Operation,
) -> syn::Item {
    let method_tokens = method_tokens(method);
    let path_lit = path;

    let op_id_doc = operation.operation_id.as_deref().map(|id| {
        let line = format!(" Operation ID: `{id}`.");
        quote! { #[doc = #line] }
    });
    let path_doc = format!(" `{} {}`", method.as_str(), path);

    parse_quote! {
        impl #request_ident {
            #[doc = #path_doc]
            #op_id_doc
            pub const METHOD: ::http::Method = #method_tokens;

            /// URL path template, with `{name}` placeholders for path
            /// parameters. Rendering into a concrete URL is the caller's
            /// responsibility.
            pub const PATH_TEMPLATE: &'static str = #path_lit;
        }
    }
}

/// Builds the runtime `MakeRequest` impl for one operation.
///
/// URL rendering substitutes path template placeholders and appends any
/// query parameters. When the operation declares a request body the
/// relevant codec encodes it and sets `Content-Type`; otherwise the
/// body defaults to [`toac::body::Body::empty`].
fn build_make_request_impl(
    request_ident: &syn::Ident,
    method: &Method,
    path: &str,
    params: &[ParamSlot],
    body: Option<&BodySlot>,
) -> syn::Item {
    let method_tokens = method_tokens(method);

    let path_rendering = render_path_statements(path, params);
    let query_rendering = render_query_statements(params);
    let header_rendering = render_header_statements(params);
    let body_apply = render_body_apply(body);
    let make_request = crate::constants::runtime_path("MakeRequest");
    let request_ty = crate::constants::runtime_path("Request");
    let body_ty = crate::constants::runtime_body_path();
    let error_ty = make_request_error_ty(body);

    parse_quote! {
        impl #make_request for #request_ident {
            type Error = #error_ty;

            fn make_request(
                self,
            ) -> impl ::std::future::Future<
                Output = ::std::result::Result<#request_ty, Self::Error>,
            > + Send {
                async move {
                    let mut __path = ::std::string::String::new();
                    #path_rendering
                    #query_rendering

                    let mut __builder = ::http::Request::builder()
                        .method(#method_tokens)
                        .uri(__path);
                    #header_rendering

                    let __request = __builder
                        .body(#body_ty::empty())
                        .expect("valid generated HTTP request");
                    #body_apply
                }
            }
        }
    }
}

/// Error type used by the generated `MakeRequest` impl.
///
/// Operations without a request body cannot fail during encoding, so
/// their `Error` associated type is [`::std::convert::Infallible`].
/// Operations with a JSON body propagate `serde_json::Error`.
fn make_request_error_ty(body: Option<&BodySlot>) -> syn::Type {
    if body.is_some() {
        parse_quote!(::serde_json::Error)
    } else {
        parse_quote!(::std::convert::Infallible)
    }
}

/// Builds the runtime `Operation` impl that links a request type to its
/// response enum. The runtime uses [`toac::body::Body`] for every
/// request, so the impl only needs to name the response enum.
fn build_operation_impl(request_ident: &syn::Ident, response_ident: &syn::Ident) -> syn::Item {
    let operation_trait = crate::constants::runtime_path("Operation");
    parse_quote! {
        impl #operation_trait for #request_ident {
            type Response = #response_ident;
        }
    }
}

/// Builds the runtime `ParseResponse` impl for one operation.
///
/// The impl consumes the fixed [`toac::Response`] and dispatches on
/// status. The body is collected before dispatch only when at least one
/// variant carries a JSON-decoded payload. Known statuses decode JSON
/// into their variant payload; unmatched statuses map to
/// [`DecodeError::UnexpectedStatus`] unless the operation declares a
/// `default` response.
fn build_parse_response_impl(
    response_ident: &syn::Ident,
    variants: &[ResponseVariant],
) -> syn::Item {
    let parse_response = crate::constants::runtime_path("ParseResponse");
    let decode_error = crate::constants::runtime_path("DecodeError");
    let box_error = crate::constants::runtime_path("BoxError");

    let arms: Vec<proc_macro2::TokenStream> =
        variants.iter().filter_map(response_match_arm).collect();
    let default_arm = variants
        .iter()
        .find(|v| v.status.eq_ignore_ascii_case("default"));
    let fallback = match default_arm {
        Some(variant) => default_fallback_tokens(response_ident, variant),
        None => quote! {
            ::std::mem::drop(__body);
            return ::std::result::Result::Err(
                #decode_error::UnexpectedStatus(__status),
            );
        },
    };
    let arm_tokens = if arms.is_empty() {
        quote! {}
    } else {
        quote! {
            match __status.as_u16() {
                #(#arms,)*
                _ => {}
            }
        }
    };

    // When the enum is the `Empty`-only placeholder (no responses
    // declared), always return that variant without touching the body.
    let empty_fallback = if variants.is_empty() {
        quote! {
            ::std::mem::drop(__body);
            return ::std::result::Result::Ok(#response_ident::Empty);
        }
    } else {
        quote! {}
    };

    parse_quote! {
        impl #parse_response for #response_ident {
            type Error = #decode_error;

            fn parse_response<__B>(
                response: ::http::Response<__B>,
            ) -> impl ::std::future::Future<
                Output = ::std::result::Result<Self, Self::Error>,
            > + ::std::marker::Send
            where
                __B: ::http_body::Body<Data = ::bytes::Bytes>
                    + ::std::marker::Send
                    + 'static,
                __B::Error: ::std::convert::Into<#box_error>,
            {
                async move {
                    let (__parts, __body) = response.into_parts();
                    let __status = __parts.status;
                    #empty_fallback
                    #arm_tokens
                    #fallback
                }
            }
        }
    }
}

/// Statements that append the path template into `__path`, substituting
/// placeholders with the string form of path-parameter fields.
fn render_path_statements(path: &str, params: &[ParamSlot]) -> proc_macro2::TokenStream {
    let mut stmts = proc_macro2::TokenStream::new();
    let path_params: std::collections::BTreeMap<&str, &syn::Ident> = params
        .iter()
        .filter(|p| p.parameter.location == ParameterIn::Path)
        .map(|p| (p.parameter.name.as_str(), &p.field_ident))
        .collect();

    for segment in path_template_segments(path) {
        match segment {
            PathSegment::Literal(lit) => {
                let lit_str = lit.to_owned();
                stmts.extend(quote! {
                    __path.push_str(#lit_str);
                });
            }
            PathSegment::Placeholder(name) => {
                if let Some(field) = path_params.get(name) {
                    stmts.extend(quote! {
                        __path.push_str(&::std::string::ToString::to_string(&self.#field));
                    });
                } else {
                    // The spec declared `{name}` in the path but never
                    // provided a matching parameter definition; leave the
                    // placeholder verbatim so the failure is visible.
                    let verbatim = format!("{{{name}}}");
                    stmts.extend(quote! {
                        __path.push_str(#verbatim);
                    });
                }
            }
        }
    }
    stmts
}

/// Statements that append query parameters to `__path`.
fn render_query_statements(params: &[ParamSlot]) -> proc_macro2::TokenStream {
    let queries: Vec<&ParamSlot> = params
        .iter()
        .filter(|p| p.parameter.location == ParameterIn::Query)
        .collect();
    if queries.is_empty() {
        return proc_macro2::TokenStream::new();
    }

    let append_each: Vec<proc_macro2::TokenStream> =
        queries.iter().map(|slot| render_one_query(slot)).collect();

    quote! {
        let mut __query_first = true;
        #(#append_each)*
    }
}

fn render_one_query(slot: &ParamSlot) -> proc_macro2::TokenStream {
    let field = &slot.field_ident;
    let wire = slot.parameter.name.as_str();
    // Scalar fields stringify directly; array-shaped fields expand to
    // repeated `?key=a&key=b` entries, matching OAS's default
    // `style=form, explode=true` for query parameters. A full
    // `style`/`explode` implementation is still TODO.
    let append = if is_vec_type(&slot.inner_ty) {
        quote! {
            for __item in __value.iter() {
                let __sep = if __query_first { '?' } else { '&' };
                __query_first = false;
                __path.push(__sep);
                __path.push_str(#wire);
                __path.push('=');
                __path.push_str(&::std::string::ToString::to_string(__item));
            }
        }
    } else {
        quote! {
            let __sep = if __query_first { '?' } else { '&' };
            __query_first = false;
            __path.push(__sep);
            __path.push_str(#wire);
            __path.push('=');
            __path.push_str(&::std::string::ToString::to_string(&__value));
        }
    };

    if slot.is_required() {
        quote! {
            {
                let __value = &self.#field;
                #append
            }
        }
    } else {
        quote! {
            if let ::std::option::Option::Some(__value) = &self.#field {
                #append
            }
        }
    }
}

/// Returns `true` if `ty` is a `Vec<_>` (plain or path-qualified).
fn is_vec_type(ty: &syn::Type) -> bool {
    let syn::Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|seg| seg.ident == "Vec")
}

/// Statements that set header parameters on `__builder`.
fn render_header_statements(params: &[ParamSlot]) -> proc_macro2::TokenStream {
    let headers: Vec<&ParamSlot> = params
        .iter()
        .filter(|p| p.parameter.location == ParameterIn::Header)
        .collect();
    if headers.is_empty() {
        return proc_macro2::TokenStream::new();
    }

    let each: Vec<proc_macro2::TokenStream> =
        headers.iter().map(|slot| render_one_header(slot)).collect();
    quote! {
        #(#each)*
    }
}

fn render_one_header(slot: &ParamSlot) -> proc_macro2::TokenStream {
    let field = &slot.field_ident;
    let wire = slot.parameter.name.as_str();
    let set = quote! {
        __builder = __builder.header(#wire, ::std::string::ToString::to_string(&__value));
    };

    if slot.is_required() {
        quote! {
            {
                let __value = &self.#field;
                #set
            }
        }
    } else {
        quote! {
            if let ::std::option::Option::Some(__value) = &self.#field {
                #set
            }
        }
    }
}

/// Tokens that fold the request body into the pre-built `__request`.
///
/// Operations without a body leave `__request` as-is. Operations with
/// a JSON body feed it through [`toac::body::codec::encode_body`],
/// which writes the serialised bytes and sets `Content-Type`. Optional
/// bodies are skipped when absent.
fn render_body_apply(body: Option<&BodySlot>) -> proc_macro2::TokenStream {
    let encode_fn = crate::constants::runtime_body_codec_path("encode_body");
    let encoder_ty = crate::constants::runtime_body_codec_path("json::JsonEncoder");

    let Some(body) = body else {
        return quote! {
            ::std::result::Result::Ok(__request)
        };
    };

    let field = &body.ident;
    if body.required {
        quote! {
            #encode_fn(
                &<#encoder_ty as ::std::default::Default>::default(),
                &self.#field,
                __request,
            )
        }
    } else {
        quote! {
            match &self.#field {
                ::std::option::Option::Some(__payload) => #encode_fn(
                    &<#encoder_ty as ::std::default::Default>::default(),
                    __payload,
                    __request,
                ),
                ::std::option::Option::None => {
                    ::std::result::Result::Ok(__request)
                }
            }
        }
    }
}

/// Builds the match arm that dispatches one status code into its response
/// enum variant. Returns `None` for statuses that can't be expressed as a
/// single `u16` (e.g. `2XX`, `default`) so callers can handle them
/// separately.
///
/// Arms that carry a payload move `__body` into the codec decoder and
/// `return` from the async block, so `__body` stays available for the
/// fallback path when no arm matches.
fn response_match_arm(variant: &ResponseVariant) -> Option<proc_macro2::TokenStream> {
    let status: u16 = variant.status.parse().ok()?;
    let variant_ident = &variant.variant_ident;
    let body_tokens = match &variant.inner_type {
        Some(_) => decode_variant_body(quote! { Self::#variant_ident }),
        None => quote! {
            ::std::mem::drop(__body);
            return ::std::result::Result::Ok(Self::#variant_ident);
        },
    };
    Some(quote! {
        #status => { #body_tokens }
    })
}

/// Tokens that consume the body into the `default` variant when present.
fn default_fallback_tokens(
    response_ident: &syn::Ident,
    variant: &ResponseVariant,
) -> proc_macro2::TokenStream {
    let variant_ident = &variant.variant_ident;
    match &variant.inner_type {
        Some(_) => decode_variant_body(quote! { #response_ident::#variant_ident }),
        None => quote! {
            ::std::mem::drop(__body);
            return ::std::result::Result::Ok(#response_ident::#variant_ident);
        },
    }
}

/// Decodes `__body` through the JSON codec into the variant payload and
/// returns the wrapping enum value. `constructor` is the tokenised path
/// of the tuple-variant constructor (e.g. `Self::Status200` or
/// `GetPetResponse::Default`).
fn decode_variant_body(constructor: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let decode_error = crate::constants::runtime_path("DecodeError");
    let decode_body = crate::constants::runtime_body_codec_path("decode_body");
    let decoder_ty = crate::constants::runtime_body_codec_path("json::JsonDecoder");
    quote! {
        let __decoder = <#decoder_ty as ::std::default::Default>::default();
        let __value = #decode_body(&__decoder, __body)
            .await
            .map_err(|e| #decode_error::Codec(::std::convert::Into::into(e)))?;
        return ::std::result::Result::Ok(#constructor(__value));
    }
}

/// Splits a path template like `/pets/{id}/files` into its literal and
/// placeholder segments.
enum PathSegment<'a> {
    Literal(&'a str),
    Placeholder(&'a str),
}

fn path_template_segments(path: &str) -> Vec<PathSegment<'_>> {
    let mut out: Vec<PathSegment<'_>> = Vec::new();
    let mut cursor = 0usize;
    let bytes = path.as_bytes();
    while cursor < bytes.len() {
        // Find next `{` from cursor.
        let open = path[cursor..].find('{').map(|rel| cursor + rel);
        let Some(open) = open else {
            out.push(PathSegment::Literal(&path[cursor..]));
            break;
        };
        if open > cursor {
            out.push(PathSegment::Literal(&path[cursor..open]));
        }
        let Some(close_rel) = path[open..].find('}') else {
            // Unclosed `{` — treat the rest as literal.
            out.push(PathSegment::Literal(&path[open..]));
            break;
        };
        let close = open + close_rel;
        out.push(PathSegment::Placeholder(&path[open + 1..close]));
        cursor = close + 1;
    }
    out
}

fn method_tokens(method: &Method) -> proc_macro2::TokenStream {
    match *method {
        Method::GET => quote! { ::http::Method::GET },
        Method::POST => quote! { ::http::Method::POST },
        Method::PUT => quote! { ::http::Method::PUT },
        Method::DELETE => quote! { ::http::Method::DELETE },
        Method::PATCH => quote! { ::http::Method::PATCH },
        Method::HEAD => quote! { ::http::Method::HEAD },
        Method::OPTIONS => quote! { ::http::Method::OPTIONS },
        Method::TRACE => quote! { ::http::Method::TRACE },
        ref other => {
            let as_str = other.as_str();
            quote! { ::http::Method::from_bytes(#as_str.as_bytes()).expect("valid method") }
        }
    }
}

/// Picks a variant ident for an OpenAPI response key.
///
/// - `default` → `Default`
/// - `200` / `201` / … → `Status200`, ... (numeric form keeps the wire
///   status visible in the type name without guessing HTTP phrases).
/// - Anything else (e.g. `2XX`) → `Status2XX`.
fn response_variant_ident(status: &str) -> syn::Ident {
    if status.eq_ignore_ascii_case("default") {
        return make_ident(DEFAULT_RESPONSE_VARIANT);
    }
    let upper = status.to_ascii_uppercase();
    make_ident(&format!("Status{upper}"))
}
