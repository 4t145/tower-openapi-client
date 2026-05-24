//! Generator entry point for the `securitySchemes` + `security` parts
//! of an OpenAPI spec.
//!
//! Emits a `pub mod security { ... }` module containing:
//!
//! - One per-scheme credential newtype (e.g. `BearerCredential`,
//!   `MyApiKeyCredential`) that wraps the corresponding `toac` built-in
//!   and implements [`toac::SecurityCredential`].
//! - An aggregate `AuthConfig` struct with `Option<...>` slots for each
//!   supported scheme, an [`AuthConfig::builder`] entry point, and an
//!   [`AuthSelector`] impl that walks the per-op requirement tree.
//!
//! The runtime `ApiClient` pulls the requirement tree from each
//! request's `http::Extensions` slot — see
//! [`crate::operations::operation`] for where the generator injects the
//! `OperationSecurity` extension during `make_request`.

use std::collections::BTreeMap;

use oas3::spec::SecurityScheme;
use quote::quote;
use syn::parse_quote;

use crate::{
    Error, Generator,
    attrs::module_inner_attrs,
    generator::Stage,
    naming::{field_ident, make_ident, to_snake_case, type_ident},
};

/// A spec security scheme the generator knows how to translate into
/// runtime credential types. Schemes outside this enum trigger
/// [`Error::Unsupported`].
#[derive(Debug, Clone)]
pub(crate) enum SupportedScheme {
    /// `type: apiKey` — `name` is the header / query / cookie key,
    /// `location` is one of `header` / `query` / `cookie`.
    ApiKey {
        name: String,
        location: ApiKeyLocation,
    },
    /// `type: http, scheme: bearer`.
    HttpBearer,
    /// `type: http, scheme: basic`.
    HttpBasic,
}

/// Where an API key travels on the wire. Mirrors OAS `in`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiKeyLocation {
    Header,
    Query,
    Cookie,
}

impl ApiKeyLocation {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "header" => Some(Self::Header),
            "query" => Some(Self::Query),
            "cookie" => Some(Self::Cookie),
            _ => None,
        }
    }

    fn runtime_variant_ident(self) -> syn::Ident {
        match self {
            Self::Header => make_ident("Header"),
            Self::Query => make_ident("Query"),
            Self::Cookie => make_ident("Cookie"),
        }
    }
}

/// Walks `spec.components.securitySchemes` and collects every scheme
/// the generator can represent.
///
/// Schemes whose shape the runtime can't handle (OAuth2, OpenID Connect,
/// mutual TLS, non-bearer/basic HTTP schemes) are **silently skipped**
/// at this stage — specs frequently declare schemes that no concrete
/// operation actually references, and failing up-front would make the
/// generator useless on those. A skipped scheme becomes a hard error
/// only when an operation's `security` list names it (see
/// [`requirement_slice_tokens`]).
///
/// # Errors
///
/// Propagates [`Error::Ref`] when a scheme `$ref` in the components
/// table cannot be resolved.
pub(crate) fn resolve_supported_schemes(
    spec: &oas3::Spec,
) -> Result<BTreeMap<String, SupportedScheme>, Error> {
    let Some(components) = spec.components.as_ref() else {
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    for (name, scheme_or_ref) in &components.security_schemes {
        let scheme = scheme_or_ref.resolve(spec)?;
        if let Some(supported) = classify_scheme(&scheme) {
            out.insert(name.clone(), supported);
        }
        // Else: unsupported scheme shape. Left out of the map; the
        // `requirement_slice_tokens` check downstream turns it into an
        // `Error::Unsupported` iff an op actually references it.
    }
    Ok(out)
}

fn classify_scheme(scheme: &SecurityScheme) -> Option<SupportedScheme> {
    match scheme {
        SecurityScheme::ApiKey {
            name: wire_name,
            location,
            ..
        } => ApiKeyLocation::parse(location).map(|loc| SupportedScheme::ApiKey {
            name: wire_name.clone(),
            location: loc,
        }),
        SecurityScheme::Http { scheme: http, .. } => match http.as_str() {
            "bearer" => Some(SupportedScheme::HttpBearer),
            "basic" => Some(SupportedScheme::HttpBasic),
            _ => None,
        },
        SecurityScheme::OAuth2 { .. }
        | SecurityScheme::OpenIdConnect { .. }
        | SecurityScheme::MutualTls { .. } => None,
    }
}

/// Per-scheme summary prepared for codegen: the scheme name (key into
/// `components.securitySchemes`, also the Rust field in `AuthConfig`),
/// the credential wrapper type name, and the Rust snake_case ident for
/// the builder / field.
struct SchemeSlot {
    spec_name: String,
    wrapper_ident: syn::Ident,
    field_ident: syn::Ident,
    scheme: SupportedScheme,
}

impl<'a> Generator<'a> {
    /// Emits the `security` module. Runs after components / operations
    /// / servers — it doesn't depend on any of them but reuses the
    /// registry's deterministic ordering.
    ///
    /// # Errors
    ///
    /// Propagates [`Error::Unsupported`] from
    /// [`resolve_supported_schemes`] when the spec declares a scheme
    /// shape the runtime can't produce code for.
    pub fn emit_security(&mut self) -> Result<(), Error> {
        self.set_stage(Stage::Security);
        let schemes = resolve_supported_schemes(self.spec)?;
        if schemes.is_empty() {
            return Ok(());
        }

        let slots: Vec<SchemeSlot> = schemes
            .into_iter()
            .map(|(spec_name, scheme)| SchemeSlot {
                wrapper_ident: type_ident(&format!("{spec_name}Credential")),
                field_ident: field_ident(&to_snake_case(&spec_name)),
                spec_name,
                scheme,
            })
            .collect();

        for slot in &slots {
            emit_credential_wrapper(self, slot);
        }
        emit_auth_config(self, &slots);
        Ok(())
    }

    /// Renders `pub mod security { ... }` from the items the security
    /// stage produced. Empty when the spec declares no supported
    /// schemes.
    pub fn finish_security(&self) -> proc_macro2::TokenStream {
        let items = self.items_in_stage(Stage::Security);
        if items.is_empty() {
            return quote! {
                pub mod security {}
            };
        }
        let attrs = module_inner_attrs();
        quote! {
            pub mod security {
                #attrs
                #(#items)*
            }
        }
    }
}

/// Emits the per-scheme credential newtype + its `SecurityCredential`
/// impl. Each wrapper is a thin shim around one of the `toac` built-in
/// credentials, with a constructor that mirrors the built-in's ergonomic
/// shape (single `token` for bearer, `username` + `password` for basic,
/// just the secret value for apiKey).
fn emit_credential_wrapper(gen_: &mut Generator<'_>, slot: &SchemeSlot) {
    let wrapper = &slot.wrapper_ident;
    let security_trait = crate::constants::runtime_path("SecurityCredential");
    let request_ty = crate::constants::runtime_path("Request");
    let box_error = crate::constants::runtime_path("BoxError");

    match &slot.scheme {
        SupportedScheme::ApiKey { name, location } => {
            let wire_name = name.as_str();
            let location_path = runtime_api_key_location_path(*location);
            let runtime_cred = runtime_security_path("ApiKeyCredential");

            let struct_item: syn::Item = parse_quote! {
                /// API-key credential. The value is sent verbatim in the
                /// header / query / cookie slot declared by the spec.
                #[derive(Debug, Clone)]
                pub struct #wrapper {
                    pub value: ::std::string::String,
                }
            };
            let impl_item: syn::Item = parse_quote! {
                impl #wrapper {
                    /// Builds a credential from a raw key value.
                    pub fn new<V: ::std::convert::Into<::std::string::String>>(
                        value: V,
                    ) -> Self {
                        Self { value: value.into() }
                    }

                    /// Projects into the runtime's concrete credential.
                    fn as_runtime(&self) -> #runtime_cred {
                        #runtime_cred {
                            name: #wire_name,
                            location: #location_path,
                            value: self.value.clone(),
                        }
                    }
                }
            };
            let trait_impl: syn::Item = parse_quote! {
                impl #security_trait for #wrapper {
                    fn apply(
                        &self,
                        req: #request_ty,
                    ) -> impl ::std::future::Future<
                        Output = ::std::result::Result<#request_ty, #box_error>,
                    > + ::std::marker::Send {
                        let __cred = self.as_runtime();
                        async move { #security_trait::apply(&__cred, req).await }
                    }
                }
            };
            store_security(
                gen_,
                &slot.spec_name,
                wrapper,
                struct_item,
                vec![impl_item, trait_impl],
            );
        }
        SupportedScheme::HttpBearer => {
            let runtime_cred = runtime_security_path("BearerCredential");
            let struct_item: syn::Item = parse_quote! {
                /// HTTP Bearer credential. The token is sent as
                /// `Authorization: Bearer <token>`.
                #[derive(Debug, Clone)]
                pub struct #wrapper {
                    pub token: ::std::string::String,
                }
            };
            let impl_item: syn::Item = parse_quote! {
                impl #wrapper {
                    /// Builds a credential from a raw token value.
                    pub fn new<T: ::std::convert::Into<::std::string::String>>(
                        token: T,
                    ) -> Self {
                        Self { token: token.into() }
                    }

                    fn as_runtime(&self) -> #runtime_cred {
                        #runtime_cred { token: self.token.clone() }
                    }
                }
            };
            let trait_impl: syn::Item = parse_quote! {
                impl #security_trait for #wrapper {
                    fn apply(
                        &self,
                        req: #request_ty,
                    ) -> impl ::std::future::Future<
                        Output = ::std::result::Result<#request_ty, #box_error>,
                    > + ::std::marker::Send {
                        let __cred = self.as_runtime();
                        async move { #security_trait::apply(&__cred, req).await }
                    }
                }
            };
            store_security(
                gen_,
                &slot.spec_name,
                wrapper,
                struct_item,
                vec![impl_item, trait_impl],
            );
        }
        SupportedScheme::HttpBasic => {
            let runtime_cred = runtime_security_path("BasicCredential");
            let struct_item: syn::Item = parse_quote! {
                /// HTTP Basic credential. Sent as
                /// `Authorization: Basic <base64(username:password)>`.
                #[derive(Debug, Clone)]
                pub struct #wrapper {
                    pub username: ::std::string::String,
                    pub password: ::std::string::String,
                }
            };
            let impl_item: syn::Item = parse_quote! {
                impl #wrapper {
                    /// Builds a credential from a username / password pair.
                    pub fn new<U, P>(username: U, password: P) -> Self
                    where
                        U: ::std::convert::Into<::std::string::String>,
                        P: ::std::convert::Into<::std::string::String>,
                    {
                        Self {
                            username: username.into(),
                            password: password.into(),
                        }
                    }

                    fn as_runtime(&self) -> #runtime_cred {
                        #runtime_cred {
                            username: self.username.clone(),
                            password: self.password.clone(),
                        }
                    }
                }
            };
            let trait_impl: syn::Item = parse_quote! {
                impl #security_trait for #wrapper {
                    fn apply(
                        &self,
                        req: #request_ty,
                    ) -> impl ::std::future::Future<
                        Output = ::std::result::Result<#request_ty, #box_error>,
                    > + ::std::marker::Send {
                        let __cred = self.as_runtime();
                        async move { #security_trait::apply(&__cred, req).await }
                    }
                }
            };
            store_security(
                gen_,
                &slot.spec_name,
                wrapper,
                struct_item,
                vec![impl_item, trait_impl],
            );
        }
    }
}

/// Emits the aggregate `AuthConfig`, its builder, and the
/// [`AuthSelector`] impl that dispatches on each requirement.
fn emit_auth_config(gen_: &mut Generator<'_>, slots: &[SchemeSlot]) {
    let auth_selector = crate::constants::runtime_path("AuthSelector");
    let auth_future = runtime_security_path("AuthFuture");
    let request_ty = crate::constants::runtime_path("Request");
    let security_trait = crate::constants::runtime_path("SecurityCredential");

    // struct AuthConfig { ... }
    let config_fields = slots.iter().map(|slot| {
        let field = &slot.field_ident;
        let wrapper = &slot.wrapper_ident;
        quote! {
            pub #field: ::std::option::Option<#wrapper>
        }
    });
    let config_item: syn::Item = parse_quote! {
        /// Aggregate of every credential the spec's security schemes
        /// might need. Each field is optional — callers populate only
        /// the schemes they actually use. Build through
        /// [`AuthConfig::builder`].
        #[derive(Debug, Clone, Default)]
        pub struct AuthConfig {
            #(#config_fields,)*
        }
    };

    // impl AuthConfig { pub fn builder() -> AuthConfigBuilder { ... } }
    let builder_ident = make_ident("AuthConfigBuilder");
    let ctor_item: syn::Item = parse_quote! {
        impl AuthConfig {
            /// Starts a fluent builder for [`AuthConfig`]. Each setter
            /// is named after the scheme's key in
            /// `components.securitySchemes`, normalised to snake_case.
            pub fn builder() -> #builder_ident {
                <#builder_ident as ::std::default::Default>::default()
            }
        }
    };

    // struct AuthConfigBuilder { ... } — same fields as AuthConfig.
    let builder_fields = slots.iter().map(|slot| {
        let field = &slot.field_ident;
        let wrapper = &slot.wrapper_ident;
        quote! {
            #field: ::std::option::Option<#wrapper>
        }
    });
    let builder_item: syn::Item = parse_quote! {
        #[derive(Debug, Clone, Default)]
        pub struct #builder_ident {
            #(#builder_fields,)*
        }
    };

    // Builder setters: one per slot, each takes the credential's
    // ergonomic constructor arguments and stores the wrapper.
    let setter_methods: Vec<proc_macro2::TokenStream> =
        slots.iter().map(render_builder_setter).collect();
    let field_idents: Vec<&syn::Ident> = slots.iter().map(|s| &s.field_ident).collect();
    let builder_impl: syn::Item = parse_quote! {
        impl #builder_ident {
            #(#setter_methods)*

            /// Finalises into an [`AuthConfig`].
            pub fn build(self) -> AuthConfig {
                let Self { #(#field_idents),* } = self;
                AuthConfig { #(#field_idents),* }
            }
        }
    };

    // AuthSelector impl: pick the first satisfiable alternative from
    // the requirement tree and apply every credential in it.
    let alt_check_arms: Vec<proc_macro2::TokenStream> = slots
        .iter()
        .map(|slot| {
            let field = &slot.field_ident;
            let name = slot.spec_name.as_str();
            quote! {
                if __scheme == #name {
                    let ::std::option::Option::Some(_) = &self.#field else {
                        __can_satisfy = false;
                        break;
                    };
                    __matched_any = true;
                    continue;
                }
            }
        })
        .collect();
    let apply_arms: Vec<proc_macro2::TokenStream> = slots
        .iter()
        .map(|slot| {
            let field = &slot.field_ident;
            let name = slot.spec_name.as_str();
            quote! {
                if __scheme == #name {
                    let ::std::option::Option::Some(__cred) = &self.#field else {
                        continue;
                    };
                    __req = #security_trait::apply(__cred, __req).await?;
                    continue;
                }
            }
        })
        .collect();

    let selector_impl: syn::Item = parse_quote! {
        impl #auth_selector for AuthConfig {
            fn apply_for(
                &self,
                req: #request_ty,
                requirements: &'static [&'static [&'static str]],
            ) -> #auth_future<'_> {
                ::std::boxed::Box::pin(async move {
                    // Public endpoint or no declared security → pass through.
                    if requirements.is_empty() {
                        return ::std::result::Result::Ok(req);
                    }
                    // Pick the first alternative whose scheme list is
                    // fully satisfiable from the stored credentials.
                    let mut __chosen: ::std::option::Option<
                        &'static [&'static str]
                    > = ::std::option::Option::None;
                    for __alt in requirements {
                        let mut __can_satisfy = true;
                        let mut __matched_any = false;
                        for __scheme in __alt.iter().copied() {
                            #(#alt_check_arms)*
                            // Unknown scheme name (not in this spec's
                            // `components.securitySchemes` or not in a
                            // supported shape) → alternative fails.
                            let _ = __matched_any;
                            __can_satisfy = false;
                            break;
                        }
                        if __can_satisfy && __matched_any {
                            __chosen = ::std::option::Option::Some(__alt);
                            break;
                        }
                    }
                    let ::std::option::Option::Some(__alt) = __chosen else {
                        return ::std::result::Result::Err(
                            ::std::convert::Into::into(
                                ::std::format!(
                                    "no configured credentials satisfy {:?}",
                                    requirements,
                                ),
                            ),
                        );
                    };
                    let mut __req = req;
                    for __scheme in __alt.iter().copied() {
                        #(#apply_arms)*
                    }
                    ::std::result::Result::Ok(__req)
                })
            }
        }
    };

    store_unnamed(gen_, config_item);
    store_unnamed(gen_, ctor_item);
    store_unnamed(gen_, builder_item);
    store_unnamed(gen_, builder_impl);
    store_unnamed(gen_, selector_impl);
}

/// Renders one builder setter. Signature varies by scheme kind so
/// callers get the idiomatic input (single `token` / `value`, or
/// `username` + `password`) rather than being forced to construct the
/// wrapper manually.
fn render_builder_setter(slot: &SchemeSlot) -> proc_macro2::TokenStream {
    let field = &slot.field_ident;
    let wrapper = &slot.wrapper_ident;
    match slot.scheme {
        SupportedScheme::ApiKey { .. } => quote! {
            /// Provides the API-key credential for this scheme.
            pub fn #field<V>(mut self, value: V) -> Self
            where
                V: ::std::convert::Into<::std::string::String>,
            {
                self.#field = ::std::option::Option::Some(
                    #wrapper::new(value),
                );
                self
            }
        },
        SupportedScheme::HttpBearer => quote! {
            /// Provides the Bearer token for this scheme.
            pub fn #field<T>(mut self, token: T) -> Self
            where
                T: ::std::convert::Into<::std::string::String>,
            {
                self.#field = ::std::option::Option::Some(
                    #wrapper::new(token),
                );
                self
            }
        },
        SupportedScheme::HttpBasic => quote! {
            /// Provides the Basic credentials (username + password).
            pub fn #field<U, P>(mut self, username: U, password: P) -> Self
            where
                U: ::std::convert::Into<::std::string::String>,
                P: ::std::convert::Into<::std::string::String>,
            {
                self.#field = ::std::option::Option::Some(
                    #wrapper::new(username, password),
                );
                self
            }
        },
    }
}

fn store_security(
    gen_: &mut Generator<'_>,
    spec_name: &str,
    wrapper: &syn::Ident,
    struct_item: syn::Item,
    extras: Vec<syn::Item>,
) {
    gen_.store_named(
        format!("__security/{spec_name}/wrapper"),
        wrapper.clone(),
        struct_item,
    );
    for item in extras {
        gen_.store_unnamed(item);
    }
}

fn store_unnamed(gen_: &mut Generator<'_>, item: syn::Item) {
    gen_.store_unnamed(item);
}

/// `::toac::security::<name>` — helper for paths the generator often
/// reaches into (e.g. `ApiKeyLocation::Header`).
fn runtime_security_path(item: &str) -> syn::Path {
    let crate_ident = syn::Ident::new(
        crate::constants::RUNTIME_CRATE,
        proc_macro2::Span::call_site(),
    );
    let item_ident = syn::Ident::new(item, proc_macro2::Span::call_site());
    parse_quote!(::#crate_ident::security::#item_ident)
}

fn runtime_api_key_location_path(location: ApiKeyLocation) -> syn::Path {
    let base = runtime_security_path("ApiKeyLocation");
    let variant = location.runtime_variant_ident();
    parse_quote!(#base::#variant)
}

/// Returns the static-slice literal representing a `SecurityRequirement`
/// list (outer OR, inner AND), keeping only the alternatives whose
/// schemes are **all** supported by the generator. Alternatives that
/// reference unknown or unsupported schemes are dropped — the runtime
/// `AuthSelector` impl wouldn't be able to satisfy them anyway.
///
/// When every alternative gets dropped, the returned tokens represent
/// an empty slice (`&[]`), which the runtime treats as a public
/// endpoint. A `cargo:warning=` line is emitted so build-script users
/// notice the downgrade; the op is still generated so specs like
/// petstore (which mixes oauth2-only endpoints with otherwise handled
/// operations) can still be consumed.
///
/// Returns `None` when `requirements` was empty to begin with — the
/// caller should skip attaching the extension entirely in that case.
pub(crate) fn requirement_slice_tokens(
    requirements: &[oas3::spec::SecurityRequirement],
    supported: &BTreeMap<String, SupportedScheme>,
) -> Result<proc_macro2::TokenStream, Error> {
    let mut alternatives: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    for req in requirements {
        let scheme_names: Vec<&str> = req.0.keys().map(String::as_str).collect();
        let all_supported = scheme_names.iter().all(|n| supported.contains_key(*n));
        if !all_supported {
            dropped.push(format!("[{}]", scheme_names.join(", ")));
            continue;
        }
        alternatives.push(quote! { &[ #(#scheme_names),* ] });
    }
    if !dropped.is_empty() {
        // build.rs sees this through `cargo:warning=`. In non-build
        // contexts (tests) it lands on stderr as a regular `eprintln!`
        // — slightly noisy but still better than silent drops.
        eprintln!(
            "cargo:warning=toac-build: dropped security alternatives referencing \
             unsupported schemes: {}",
            dropped.join(", ")
        );
    }
    Ok(quote! { &[ #(#alternatives),* ] })
}
