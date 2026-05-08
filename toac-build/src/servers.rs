//! Generator entry point for the `servers` section of an OpenAPI spec.
//!
//! Emits one `ServerOption{index}` type per entry, plus an aggregate
//! `ApiServer` enum (or type alias when there's only one option) that
//! implements [`toac::Server`]. Users pass any of these into
//! [`toac::ApiClient::new`] — the client resolves the URL once at
//! construction and doesn't retain the concrete server type.
//!
//! When the spec declares no servers the OAS default (`url: /`) is
//! materialised as `ServerOption0`.
//!
//! Both root-level (`spec.servers`) and operation-level
//! (`operation.servers`) rendering share the same emit logic — the
//! helpers below take a `&mut Generator` and don't touch the active
//! stage, so the caller picks the module the items land in.

use oas3::spec::{Server, ServerVariable};
use quote::quote;
use syn::parse_quote;

use crate::{
    Error, Generator,
    docs::push_schema_docs,
    generator::Stage,
    naming::{field_ident, make_ident, to_pascal_case, type_ident},
};

/// URL used when a spec declares no servers (OAS default).
const ROOT_SERVER_URL: &str = "/";

impl<'a> Generator<'a> {
    /// Emits the `servers` module for this spec, covering root-level
    /// `spec.servers`.
    ///
    /// Per-operation `servers` overrides are handled during operation
    /// generation so the generated `{Op}Server` enum can sit next to
    /// the operation types.
    ///
    /// # Errors
    ///
    /// Propagates generator errors raised while rendering server items.
    pub fn emit_servers(&mut self) -> Result<(), Error> {
        self.set_stage(Stage::Servers);
        let servers = self.spec.servers.clone();
        let effective: Vec<Server> = if servers.is_empty() {
            vec![Server {
                url: ROOT_SERVER_URL.to_owned(),
                description: None,
                variables: Default::default(),
                extensions: Default::default(),
            }]
        } else {
            servers
        };

        let mut option_idents: Vec<syn::Ident> = Vec::with_capacity(effective.len());
        for (index, server) in effective.iter().enumerate() {
            let option_ident = type_ident(&format!("ServerOption{index}"));
            emit_server_option_in_stage(self, "", &option_ident, server)?;
            option_idents.push(option_ident);
        }

        self.emit_api_server(&option_idents);
        Ok(())
    }

    /// Renders `pub mod servers { ... }` from the items registered
    /// during the servers stage.
    pub fn finish_servers(&self) -> proc_macro2::TokenStream {
        let items = self.items_in_stage(Stage::Servers);
        if items.is_empty() {
            return quote! {
                pub mod servers {}
            };
        }
        quote! {
            pub mod servers {
                #(#items)*
            }
        }
    }

    /// Emits the aggregate `ApiServer` (enum of every option) with its
    /// [`toac::Server`] impl and `Default`.
    ///
    /// When only one option exists the aggregate collapses to a type
    /// alias so users don't have to match a single-variant enum.
    fn emit_api_server(&mut self, option_idents: &[syn::Ident]) {
        let aggregate_ident = type_ident("ApiServer");

        if option_idents.len() == 1 {
            let only = &option_idents[0];
            let alias: syn::Item = parse_quote! {
                pub type #aggregate_ident = #only;
            };
            self.store_named(
                format!("__server/{aggregate_ident}"),
                aggregate_ident,
                alias,
            );
            return;
        }

        emit_aggregate_in_stage(self, "", &aggregate_ident, option_idents);
    }
}

// ---------------------------------------------------------------------------
// Free-function variants reused by both the root `servers` stage and
// the per-operation servers emitted from `operations::operation`. These
// do NOT touch `set_stage`, so the caller decides which module items
// land in.
// ---------------------------------------------------------------------------

/// Emits the struct / default / impl triple for one server option
/// into whatever stage the generator is currently writing to.
///
/// `key_prefix` disambiguates registry keys when two callers would
/// otherwise emit items with the same ident (e.g. two operations both
/// hosting a `ServerOption0` under their respective path modules).
/// Pass `""` at the root-level servers stage.
///
/// # Errors
///
/// Propagates generator errors raised while rendering nested variable
/// enum types.
pub(crate) fn emit_server_option_in_stage(
    gen_: &mut Generator<'_>,
    key_prefix: &str,
    option_ident: &syn::Ident,
    server: &Server,
) -> Result<(), Error> {
    let url = server.url.as_str();
    let doc_attrs = server_doc_attrs(server);

    if server.variables.is_empty() {
        let struct_item: syn::Item = parse_quote! {
            #(#doc_attrs)*
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
            pub struct #option_ident;
        };
        let impl_item = server_impl_for_unit(option_ident, url);
        gen_.store_named(
            scoped_key("__server", key_prefix, option_ident),
            option_ident.clone(),
            struct_item,
        );
        gen_.store_unnamed(impl_item);
        return Ok(());
    }

    let mut variable_slots: Vec<VariableSlot> = Vec::with_capacity(server.variables.len());
    for (name, variable) in &server.variables {
        let slot = build_variable_slot(gen_, key_prefix, option_ident, name, variable)?;
        variable_slots.push(slot);
    }

    let field_defs = variable_slots.iter().map(|slot| {
        let ident = &slot.field_ident;
        let ty = &slot.field_type;
        let desc = slot.description.as_deref();
        let doc = desc.map(|d| quote! { #[doc = #d] });
        quote! {
            #doc
            pub #ident: #ty
        }
    });

    let struct_item: syn::Item = parse_quote! {
        #(#doc_attrs)*
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct #option_ident {
            #(#field_defs,)*
        }
    };

    let default_fields = variable_slots.iter().map(|slot| {
        let ident = &slot.field_ident;
        let default_expr = &slot.default_expr;
        quote! { #ident: #default_expr }
    });
    let default_impl: syn::Item = parse_quote! {
        impl ::std::default::Default for #option_ident {
            fn default() -> Self {
                Self {
                    #(#default_fields,)*
                }
            }
        }
    };

    let impl_item = server_impl_for_templated(option_ident, url, &variable_slots);

    gen_.store_named(
        scoped_key("__server", key_prefix, option_ident),
        option_ident.clone(),
        struct_item,
    );
    gen_.store_unnamed(default_impl);
    gen_.store_unnamed(impl_item);
    Ok(())
}

/// Emits an aggregate enum with one variant per option, plus its
/// [`toac::Server`] impl, `Default`, and `From` conversions. See
/// [`emit_server_option_in_stage`] for `key_prefix` semantics.
pub(crate) fn emit_aggregate_in_stage(
    gen_: &mut Generator<'_>,
    key_prefix: &str,
    aggregate_ident: &syn::Ident,
    option_idents: &[syn::Ident],
) {
    let server_trait = crate::constants::runtime_path("Server");

    let variants = option_idents.iter().enumerate().map(|(i, ident)| {
        let variant_ident = make_ident(&format!("Option{i}"));
        quote! { #variant_ident(#ident) }
    });
    let enum_item: syn::Item = parse_quote! {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum #aggregate_ident {
            #(#variants,)*
        }
    };

    let base_url_arms = option_idents.iter().enumerate().map(|(i, _)| {
        let variant_ident = make_ident(&format!("Option{i}"));
        quote! {
            Self::#variant_ident(__s) => #server_trait::base_url(__s)
        }
    });
    let impl_item: syn::Item = parse_quote! {
        impl #server_trait for #aggregate_ident {
            fn base_url(&self) -> ::std::borrow::Cow<'_, str> {
                match self {
                    #(#base_url_arms,)*
                }
            }
        }
    };

    let first = &option_idents[0];
    let default_impl: syn::Item = parse_quote! {
        impl ::std::default::Default for #aggregate_ident {
            fn default() -> Self {
                Self::Option0(<#first as ::std::default::Default>::default())
            }
        }
    };

    for (i, ident) in option_idents.iter().enumerate() {
        let variant_ident = make_ident(&format!("Option{i}"));
        let from_impl: syn::Item = parse_quote! {
            impl ::std::convert::From<#ident> for #aggregate_ident {
                fn from(value: #ident) -> Self {
                    Self::#variant_ident(value)
                }
            }
        };
        gen_.store_unnamed(from_impl);
    }

    gen_.store_named(
        scoped_key("__server", key_prefix, aggregate_ident),
        aggregate_ident.clone(),
        enum_item,
    );
    gen_.store_unnamed(impl_item);
    gen_.store_unnamed(default_impl);
}

/// Composes a globally-unique registry key from a stage-specific
/// namespace, an optional module-path qualifier, and the item's ident.
fn scoped_key(namespace: &str, key_prefix: &str, ident: &syn::Ident) -> String {
    if key_prefix.is_empty() {
        format!("{namespace}/{ident}")
    } else {
        format!("{namespace}/{key_prefix}/{ident}")
    }
}

/// Fully-resolved description of one server variable slot.
struct VariableSlot {
    field_ident: syn::Ident,
    field_type: syn::Type,
    default_expr: syn::Expr,
    description: Option<String>,
}

fn build_variable_slot(
    gen_: &mut Generator<'_>,
    key_prefix: &str,
    parent: &syn::Ident,
    name: &str,
    variable: &ServerVariable,
) -> Result<VariableSlot, Error> {
    let field_ident = field_ident(name);

    if variable.substitutions_enum.is_empty() {
        let lit = &variable.default;
        return Ok(VariableSlot {
            field_ident,
            field_type: parse_quote!(::std::string::String),
            default_expr: parse_quote!(::std::string::String::from(#lit)),
            description: variable.description.clone(),
        });
    }

    let enum_ident = type_ident(&format!("{parent}{}", to_pascal_case(name)));
    let mut variants: Vec<(syn::Ident, String)> =
        Vec::with_capacity(variable.substitutions_enum.len());
    for raw in &variable.substitutions_enum {
        let variant_ident = make_ident(&to_pascal_case(raw));
        variants.push((variant_ident, raw.clone()));
    }

    let variant_tokens = variants.iter().map(|(ident, raw)| {
        let canonical = ident.to_string();
        let canonical = canonical.strip_prefix("r#").unwrap_or(&canonical);
        let rename = (canonical != raw).then(|| {
            quote! { #[serde(rename = #raw)] }
        });
        quote! {
            #rename
            #ident
        }
    });

    let display_arms = variants.iter().map(|(ident, raw)| {
        quote! {
            Self::#ident => ::std::write!(__f, #raw)
        }
    });

    let default_variant = variants
        .iter()
        .find(|(_, raw)| raw == &variable.default)
        .map(|(ident, _)| ident.clone())
        .unwrap_or_else(|| variants[0].0.clone());

    let enum_item: syn::Item = parse_quote! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
        pub enum #enum_ident {
            #(#variant_tokens,)*
        }
    };
    let display_impl: syn::Item = parse_quote! {
        impl ::std::fmt::Display for #enum_ident {
            fn fmt(&self, __f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                match self {
                    #(#display_arms,)*
                }
            }
        }
    };
    let default_impl: syn::Item = parse_quote! {
        impl ::std::default::Default for #enum_ident {
            fn default() -> Self {
                Self::#default_variant
            }
        }
    };

    gen_.store_named(
        scoped_key("__server_var", key_prefix, &enum_ident),
        enum_ident.clone(),
        enum_item,
    );
    gen_.store_unnamed(display_impl);
    gen_.store_unnamed(default_impl);

    Ok(VariableSlot {
        field_ident,
        field_type: parse_quote!(#enum_ident),
        default_expr: parse_quote!(<#enum_ident as ::std::default::Default>::default()),
        description: variable.description.clone(),
    })
}

/// Builds the `toac::Server` impl for a unit server with a constant URL.
fn server_impl_for_unit(option_ident: &syn::Ident, url: &str) -> syn::Item {
    let server_trait = crate::constants::runtime_path("Server");
    parse_quote! {
        impl #server_trait for #option_ident {
            fn base_url(&self) -> ::std::borrow::Cow<'_, str> {
                ::std::borrow::Cow::Borrowed(#url)
            }
        }
    }
}

/// Builds the `toac::Server` impl for a server whose URL carries
/// `{variable}` placeholders.
fn server_impl_for_templated(
    option_ident: &syn::Ident,
    url: &str,
    slots: &[VariableSlot],
) -> syn::Item {
    let server_trait = crate::constants::runtime_path("Server");

    let mut fmt = String::new();
    let mut args: Vec<proc_macro2::TokenStream> = Vec::new();
    for seg in template_segments(url) {
        match seg {
            TemplateSegment::Literal(lit) => {
                // Escape `{` / `}` so format_args sees them literally.
                for ch in lit.chars() {
                    if ch == '{' || ch == '}' {
                        fmt.push(ch);
                        fmt.push(ch);
                    } else {
                        fmt.push(ch);
                    }
                }
            }
            TemplateSegment::Placeholder(name) => {
                match slots.iter().find(|s| {
                    let ident = crate::naming::field_ident(name);
                    ident == s.field_ident
                }) {
                    Some(slot) => {
                        fmt.push_str("{}");
                        let ident = &slot.field_ident;
                        args.push(quote! { self.#ident });
                    }
                    None => {
                        // Placeholder without a matching variable; leave verbatim.
                        fmt.push('{');
                        fmt.push('{');
                        fmt.push_str(name);
                        fmt.push('}');
                        fmt.push('}');
                    }
                }
            }
        }
    }

    parse_quote! {
        impl #server_trait for #option_ident {
            fn base_url(&self) -> ::std::borrow::Cow<'_, str> {
                ::std::borrow::Cow::Owned(::std::format!(#fmt, #(#args),*))
            }
        }
    }
}

/// Builds the documentation attributes for one server entry (url +
/// description).
fn server_doc_attrs(server: &Server) -> Vec<syn::Attribute> {
    let mut attrs: Vec<syn::Attribute> = Vec::new();
    let url_doc = format!(" `{}`", server.url);
    attrs.push(parse_quote!(#[doc = #url_doc]));
    push_schema_docs(&mut attrs, None, server.description.as_deref(), &[]);
    attrs
}

/// Splits a URL template into literal and placeholder segments.
enum TemplateSegment<'a> {
    Literal(&'a str),
    Placeholder(&'a str),
}

fn template_segments(url: &str) -> Vec<TemplateSegment<'_>> {
    let mut out: Vec<TemplateSegment<'_>> = Vec::new();
    let mut cursor = 0;
    while cursor < url.len() {
        let rest = &url[cursor..];
        let Some(open_rel) = rest.find('{') else {
            out.push(TemplateSegment::Literal(&url[cursor..]));
            break;
        };
        let open = cursor + open_rel;
        if open > cursor {
            out.push(TemplateSegment::Literal(&url[cursor..open]));
        }
        let Some(close_rel) = url[open..].find('}') else {
            out.push(TemplateSegment::Literal(&url[open..]));
            break;
        };
        let close = open + close_rel;
        out.push(TemplateSegment::Placeholder(&url[open + 1..close]));
        cursor = close + 1;
    }
    out
}
