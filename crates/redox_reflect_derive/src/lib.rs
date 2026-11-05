//! `#[derive(Reflect)]` for `redox_reflect`.
//!
//! ```ignore
//! #[derive(Reflect, Default)]
//! #[reflect(name = "Health", category = "Gameplay", requires("Transform"), version = 2)]
//! struct Health {
//!     #[reflect(range(min = 0.0, max = 1.0), step = 0.01, unit = "m", default = 1.0)]
//!     current: f32,
//!     #[reflect(read_only, widget = "color")]
//!     tint: f32,
//!     #[reflect(skip)]             // not exposed to the inspector or saved
//!     cache: Vec<u8>,
//! }
//! ```
//!
//! Every non-skipped field type must implement `redox_reflect::FieldValue`.
//! Only structs with named fields are supported.
//!
//! Field attributes: `skip`, `label = "…"`, `category = "…"`, `tooltip = "…"`,
//! `range(min = …, max = …)`, `step = …`, `unit = "…"`, `default = <literal>`, `read_only`,
//! `hidden`, `advanced`, `editor_only`, `widget = "auto" | "color" | "angle" | "slider" |
//! "multiline" | "path"`.
//!
//! Type attributes: `name = "…"`, `category = "…"`, `tooltip = "…"`, `requires("…", …)`,
//! `version = <integer>`, `editor_only`.
//!
//! A `default` value must be a literal (number, `bool` or string); other expressions are errors.

#![warn(missing_docs)]

use std::collections::BTreeMap;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::meta::ParseNestedMeta;
use syn::punctuated::Punctuated;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, ExprUnary, Fields, Lit, LitInt, LitStr, Token,
    UnOp, parenthesized, parse_macro_input,
};

/// Attribute names accepted on a type.
const TYPE_KEYS: &[&str] = &[
    "name",
    "category",
    "tooltip",
    "requires",
    "version",
    "editor_only",
];

/// Attribute names accepted on a field.
const FIELD_KEYS: &[&str] = &[
    "skip",
    "label",
    "category",
    "tooltip",
    "range",
    "step",
    "unit",
    "default",
    "read_only",
    "hidden",
    "advanced",
    "editor_only",
    "widget",
    "accepts",
];

/// Derives `redox_reflect::Reflect` for a struct with named fields.
#[proc_macro_derive(Reflect, attributes(reflect))]
pub fn derive_reflect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Parsed `#[reflect(...)]` attributes of a type.
#[derive(Default)]
struct TypeAttrs {
    name: Option<String>,
    /// Tokens of each `TypeMeta` field, keyed by field name.
    meta: BTreeMap<String, TokenStream2>,
}

/// Parsed `#[reflect(...)]` attributes of a field.
#[derive(Default)]
struct FieldAttrs {
    skip: bool,
    /// Tokens of each `FieldMeta` field, keyed by field name.
    meta: BTreeMap<String, TokenStream2>,
}

/// Returns the attribute name, or an error listing the accepted names.
fn key_of(meta: &ParseNestedMeta<'_>, allowed: &[&str]) -> syn::Result<String> {
    let key = meta
        .path
        .get_ident()
        .map(|ident| ident.to_string())
        .unwrap_or_default();
    if allowed.contains(&key.as_str()) {
        Ok(key)
    } else {
        Err(meta.error(format!(
            "unknown `reflect` attribute `{key}`; expected one of: {}",
            allowed.join(", ")
        )))
    }
}

fn parse_type_attrs(attrs: &[Attribute]) -> syn::Result<TypeAttrs> {
    let mut out = TypeAttrs::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("reflect")) {
        attr.parse_nested_meta(|meta| {
            let key = key_of(&meta, TYPE_KEYS)?;
            let value = match key.as_str() {
                "name" => {
                    let lit: LitStr = meta.value()?.parse()?;
                    out.name = Some(lit.value());
                    return Ok(());
                }
                "requires" => string_list_tokens(&meta)?,
                "version" => {
                    let lit: LitInt = meta.value()?.parse()?;
                    let _: u32 = lit.base10_parse()?;
                    quote!(#lit)
                }
                "editor_only" => quote!(true),
                // category, tooltip
                _ => {
                    let lit: LitStr = meta.value()?.parse()?;
                    quote!(Some(#lit))
                }
            };
            out.meta.insert(key, value);
            Ok(())
        })?;
    }
    Ok(out)
}

fn parse_field_attrs(attrs: &[Attribute]) -> syn::Result<FieldAttrs> {
    let mut out = FieldAttrs::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("reflect")) {
        attr.parse_nested_meta(|meta| {
            let key = key_of(&meta, FIELD_KEYS)?;
            let value = match key.as_str() {
                "skip" => {
                    out.skip = true;
                    return Ok(());
                }
                "read_only" | "hidden" | "advanced" | "editor_only" => quote!(true),
                "range" => {
                    let (min, max) = range_values(&meta)?;
                    quote!(Some((#min, #max)))
                }
                "step" => {
                    let expr: Expr = meta.value()?.parse()?;
                    let step = number_of(&expr)?;
                    quote!(Some(#step))
                }
                "default" => {
                    let expr: Expr = meta.value()?.parse()?;
                    let value = reflect_value_tokens(&expr)?;
                    quote!(Some(#value))
                }
                "widget" => {
                    let lit: LitStr = meta.value()?.parse()?;
                    let widget = widget_tokens(&lit)?;
                    quote!(#widget)
                }
                "accepts" => string_list_tokens(&meta)?,
                // label, category, tooltip, unit
                _ => {
                    let lit: LitStr = meta.value()?.parse()?;
                    quote!(Some(#lit))
                }
            };
            out.meta.insert(key, value);
            Ok(())
        })?;
    }
    Ok(out)
}

/// A parenthesised list of strings, such as `requires("A", "B")` or `accepts("glb", "gltf")`.
fn string_list_tokens(meta: &ParseNestedMeta<'_>) -> syn::Result<TokenStream2> {
    let content;
    parenthesized!(content in meta.input);
    let names = Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?;
    let names: Vec<LitStr> = names.into_iter().collect();
    Ok(quote!(&[#(#names),*]))
}

/// `range(min = …, max = …)` on a field.
fn range_values(meta: &ParseNestedMeta<'_>) -> syn::Result<(f64, f64)> {
    let mut min = None;
    let mut max = None;
    meta.parse_nested_meta(|inner| {
        let key = key_of(&inner, &["min", "max"])?;
        let expr: Expr = inner.value()?.parse()?;
        let number = number_of(&expr)?;
        if key == "min" {
            min = Some(number);
        } else {
            max = Some(number);
        }
        Ok(())
    })?;
    match (min, max) {
        (Some(min), Some(max)) => Ok((min, max)),
        _ => Err(meta.error("`range` needs both `min` and `max`")),
    }
}

/// A number literal, optionally negative.
fn number_of(expr: &Expr) -> syn::Result<f64> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Float(float),
            ..
        }) => float.base10_parse(),
        Expr::Lit(ExprLit {
            lit: Lit::Int(int), ..
        }) => Ok(int.base10_parse::<i64>()? as f64),
        Expr::Unary(ExprUnary {
            op: UnOp::Neg(_),
            expr: inner,
            ..
        }) => Ok(-number_of(inner)?),
        _ => Err(syn::Error::new_spanned(expr, "expected a number literal")),
    }
}

/// A literal `default` value as a `ReflectValue` expression.
fn reflect_value_tokens(expr: &Expr) -> syn::Result<TokenStream2> {
    match expr {
        Expr::Lit(ExprLit { lit, .. }) => literal_value_tokens(lit, false),
        Expr::Unary(ExprUnary {
            op: UnOp::Neg(_),
            expr: inner,
            ..
        }) => match &**inner {
            Expr::Lit(ExprLit { lit, .. }) => literal_value_tokens(lit, true),
            _ => Err(non_literal_default(expr)),
        },
        _ => Err(non_literal_default(expr)),
    }
}

fn non_literal_default(expr: &Expr) -> syn::Error {
    syn::Error::new_spanned(
        expr,
        "`default` accepts only literals (number, bool or string); use Reset or a manual impl for other values",
    )
}

fn literal_value_tokens(lit: &Lit, negative: bool) -> syn::Result<TokenStream2> {
    match lit {
        Lit::Float(float) => {
            let value: f64 = float.base10_parse()?;
            let value = if negative { -value } else { value };
            Ok(quote!(::redox_reflect::ReflectValue::F64(#value)))
        }
        Lit::Int(int) => {
            let value: i64 = int.base10_parse()?;
            let value = if negative { -value } else { value };
            Ok(quote!(::redox_reflect::ReflectValue::I64(#value)))
        }
        Lit::Bool(boolean) if !negative => {
            let value = boolean.value;
            Ok(quote!(::redox_reflect::ReflectValue::Bool(#value)))
        }
        Lit::Str(text) if !negative => {
            Ok(quote!(::redox_reflect::ReflectValue::String(#text.to_string())))
        }
        _ => Err(syn::Error::new_spanned(
            lit,
            "`default` accepts only literals (number, bool or string)",
        )),
    }
}

fn widget_tokens(lit: &LitStr) -> syn::Result<TokenStream2> {
    let variant = match lit.value().as_str() {
        "auto" => "Auto",
        "color" => "Color",
        "angle" => "Angle",
        "slider" => "Slider",
        "multiline" => "Multiline",
        "path" => "Path",
        other => {
            return Err(syn::Error::new_spanned(
                lit,
                format!(
                    "unknown widget `{other}`; expected auto, color, angle, slider, multiline or path"
                ),
            ));
        }
    };
    let variant = format_ident!("{}", variant);
    Ok(quote!(::redox_reflect::meta::WidgetHint::#variant))
}

/// A `FieldMeta` or `TypeMeta` struct literal; keys not given take their `Default` value.
fn meta_struct(path: TokenStream2, fields: &BTreeMap<String, TokenStream2>) -> TokenStream2 {
    let assigns: Vec<TokenStream2> = fields
        .iter()
        .map(|(key, value)| {
            let key = format_ident!("{}", key);
            quote!(#key: #value)
        })
        .collect();
    quote!({
        #[allow(clippy::needless_update)]
        let meta = #path { #(#assigns,)* ..::core::default::Default::default() };
        meta
    })
}

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "`#[derive(Reflect)]` only supports structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "`#[derive(Reflect)]` needs a struct with named fields",
        ));
    };

    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let type_attrs = parse_type_attrs(&input.attrs)?;
    let type_name = type_attrs.name.unwrap_or_else(|| ident.to_string());
    let type_meta = meta_struct(quote!(::redox_reflect::meta::TypeMeta), &type_attrs.meta);

    let mut to_fields = Vec::new();
    let mut apply_fields = Vec::new();
    let mut schema_fields = Vec::new();
    for field in &fields.named {
        let attrs = parse_field_attrs(&field.attrs)?;
        if attrs.skip {
            continue;
        }
        let Some(name) = &field.ident else { continue };
        let name_str = name.to_string();
        let ty = &field.ty;
        let field_meta = meta_struct(quote!(::redox_reflect::meta::FieldMeta), &attrs.meta);
        to_fields.push(quote! {
            (#name_str.into(), ::redox_reflect::FieldValue::to_field_value(&self.#name))
        });
        apply_fields.push(quote! {
            if let Some(v) = value.field(#name_str) {
                ::redox_reflect::FieldValue::apply_field_value(&mut self.#name, v);
            }
        });
        schema_fields.push(quote! {
            ::redox_reflect::ReflectField {
                name: #name_str,
                type_name: <#ty as ::redox_reflect::FieldValue>::TYPE_NAME,
                meta: #field_meta,
            }
        });
    }

    Ok(quote! {
        impl #impl_generics ::redox_reflect::Reflect for #ident #ty_generics #where_clause {
            fn type_name() -> &'static str {
                #type_name
            }
            fn to_value(&self) -> ::redox_reflect::ReflectValue {
                ::redox_reflect::ReflectValue::Struct {
                    type_name: #type_name.into(),
                    fields: vec![ #(#to_fields),* ],
                }
            }
            fn apply(
                &mut self,
                value: &::redox_reflect::ReflectValue,
            ) -> ::std::result::Result<(), ::redox_reflect::ReflectError> {
                #(#apply_fields)*
                Ok(())
            }
            fn reflect_schema() -> ::redox_reflect::ReflectSchema {
                ::redox_reflect::ReflectSchema {
                    type_name: #type_name,
                    fields: vec![ #(#schema_fields),* ],
                    meta: #type_meta,
                }
            }
        }
    })
}
