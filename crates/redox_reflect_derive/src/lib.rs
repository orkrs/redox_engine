//! `#[derive(Reflect)]` for `redox_reflect`.
//!
//! ```ignore
//! #[derive(Reflect, Default)]
//! #[reflect(name = "Health")]      // optional: registered name, defaults to the type name
//! struct Health {
//!     current: f32,
//!     #[reflect(skip)]             // not exposed to the inspector or saved
//!     cache: Vec<u8>,
//! }
//! ```
//!
//! Every non-skipped field type must implement `redox_reflect::FieldValue`.
//! Only structs with named fields are supported.

#![warn(missing_docs)]

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, LitStr, parse_macro_input};

/// Derives `redox_reflect::Reflect` for a struct with named fields.
#[proc_macro_derive(Reflect, attributes(reflect))]
pub fn derive_reflect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn has_flag(attrs: &[syn::Attribute], flag: &str) -> syn::Result<bool> {
    let mut found = false;
    for attr in attrs.iter().filter(|a| a.path().is_ident("reflect")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident(flag) {
                found = true;
            } else if meta.path.is_ident("name") {
                // consumed by `type_name_override`
                let _: LitStr = meta.value()?.parse()?;
            } else {
                return Err(meta.error("unknown `reflect` attribute"));
            }
            Ok(())
        })?;
    }
    Ok(found)
}

fn type_name_override(attrs: &[syn::Attribute]) -> syn::Result<Option<String>> {
    let mut name = None;
    for attr in attrs.iter().filter(|a| a.path().is_ident("reflect")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                let lit: LitStr = meta.value()?.parse()?;
                name = Some(lit.value());
            } else if !meta.path.is_ident("skip") {
                return Err(meta.error("unknown `reflect` attribute"));
            }
            Ok(())
        })?;
    }
    Ok(name)
}

fn expand(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
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
    let type_name = type_name_override(&input.attrs)?.unwrap_or_else(|| ident.to_string());

    let mut to_fields = Vec::new();
    let mut apply_fields = Vec::new();
    let mut schema_fields = Vec::new();
    for field in &fields.named {
        if has_flag(&field.attrs, "skip")? {
            continue;
        }
        let Some(name) = &field.ident else { continue };
        let name_str = name.to_string();
        let ty = &field.ty;
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
                }
            }
        }
    })
}
