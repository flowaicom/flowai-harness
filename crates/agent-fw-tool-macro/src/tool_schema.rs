//! Implementation of `#[derive(ToolSchema)]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, GenericArgument, Lit, Meta, PathArguments, Type};

/// Main expansion for `#[derive(ToolSchema)]`.
pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    name,
                    "ToolSchema can only be derived for structs with named fields",
                ))
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "ToolSchema can only be derived for structs",
            ))
        }
    };

    // Parse container-level serde attributes
    let rename_all = parse_serde_rename_all(&input.attrs);

    let mut property_tokens = Vec::new();
    let mut required_tokens = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();

        // Determine the serialized field name
        let serialized_name = field_serde_rename(&field.attrs)
            .unwrap_or_else(|| apply_rename_all(&field_name_str, rename_all.as_deref()));

        // Parse #[schema(...)] attributes
        let schema_attrs = parse_schema_attrs(&field.attrs)?;

        // Get doc comment as description
        let description = schema_attrs
            .description
            .clone()
            .or_else(|| extract_doc_comment(&field.attrs));

        // Determine type info
        let (is_option, inner_type) = unwrap_option(&field.ty);
        let type_schema = type_to_schema_tokens(inner_type);

        // Build property schema
        let desc_token = if let Some(desc) = &description {
            quote! { prop.insert("description".to_string(), serde_json::Value::String(#desc.to_string())); }
        } else {
            quote! {}
        };

        let enum_token = if !schema_attrs.enum_values.is_empty() {
            let values = &schema_attrs.enum_values;
            quote! {
                prop.insert("enum".to_string(), serde_json::json!([#(#values),*]));
            }
        } else {
            quote! {}
        };

        property_tokens.push(quote! {
            {
                let mut prop = #type_schema;
                #desc_token
                #enum_token
                properties.insert(#serialized_name.to_string(), serde_json::Value::Object(prop));
            }
        });

        // Required: non-Option fields OR fields with #[schema(required)]
        if !is_option || schema_attrs.required {
            required_tokens.push(quote! {
                required.push(serde_json::Value::String(#serialized_name.to_string()));
            });
        }
    }

    Ok(quote! {
        impl agent_fw_tool::ToolSchema for #name {
            fn json_schema() -> serde_json::Value {
                let mut properties = serde_json::Map::new();
                let mut required = Vec::<serde_json::Value>::new();

                #(#property_tokens)*
                #(#required_tokens)*

                let mut schema = serde_json::Map::new();
                schema.insert("type".to_string(), serde_json::Value::String("object".to_string()));
                schema.insert("properties".to_string(), serde_json::Value::Object(properties));
                if !required.is_empty() {
                    schema.insert("required".to_string(), serde_json::Value::Array(required));
                }
                serde_json::Value::Object(schema)
            }
        }
    })
}

/// Parsed `#[schema(...)]` attributes on a field.
#[derive(Default)]
struct SchemaAttrs {
    required: bool,
    description: Option<String>,
    enum_values: Vec<String>,
}

fn parse_schema_attrs(attrs: &[syn::Attribute]) -> syn::Result<SchemaAttrs> {
    let mut result = SchemaAttrs::default();

    for attr in attrs {
        if !attr.path().is_ident("schema") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("required") {
                result.required = true;
                Ok(())
            } else if meta.path.is_ident("description") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                result.description = Some(lit.value());
                Ok(())
            } else if meta.path.is_ident("enum_values") {
                let value = meta.value()?;
                let content;
                syn::bracketed!(content in value);
                let values = content.parse_terminated(
                    |input: syn::parse::ParseStream| input.parse::<syn::LitStr>(),
                    syn::Token![,],
                )?;
                for lit in values {
                    result.enum_values.push(lit.value());
                }
                Ok(())
            } else {
                Err(meta.error("unknown schema attribute"))
            }
        })?;
    }

    Ok(result)
}

/// Extract doc comment from attributes.
fn extract_doc_comment(attrs: &[syn::Attribute]) -> Option<String> {
    let mut lines = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let Meta::NameValue(nv) = &attr.meta {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: Lit::Str(s), ..
                }) = &nv.value
                {
                    lines.push(s.value().trim().to_string());
                }
            }
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join(" "))
    }
}

/// Parse `#[serde(rename_all = "...")]` from container attributes.
fn parse_serde_rename_all(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let mut rename_all = None;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                if let Lit::Str(s) = lit {
                    rename_all = Some(s.value());
                }
            }
            Ok(())
        });
        if rename_all.is_some() {
            return rename_all;
        }
    }
    None
}

/// Parse `#[serde(rename = "...")]` from field attributes.
fn field_serde_rename(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let mut rename = None;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                if let Lit::Str(s) = lit {
                    rename = Some(s.value());
                }
            }
            Ok(())
        });
        if rename.is_some() {
            return rename;
        }
    }
    None
}

/// Apply rename_all transformation.
fn apply_rename_all(name: &str, rename_all: Option<&str>) -> String {
    match rename_all {
        Some("camelCase") => to_camel_case(name),
        Some("snake_case") => name.to_string(),
        Some("SCREAMING_SNAKE_CASE") => name.to_uppercase(),
        Some("kebab-case") => name.replace('_', "-"),
        Some("PascalCase") => to_pascal_case(name),
        _ => name.to_string(),
    }
}

fn to_camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;
    for (i, c) in s.chars().enumerate() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else if i == 0 {
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;
    for c in s.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }
    result
}

/// Check if a type is `Option<T>` and return the inner type.
fn unwrap_option(ty: &Type) -> (bool, &Type) {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Option" {
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(GenericArgument::Type(inner)) = args.args.first() {
                        return (true, inner);
                    }
                }
            }
        }
    }
    (false, ty)
}

/// Check if a type is `Vec<T>` and return the inner type.
fn unwrap_vec(ty: &Type) -> Option<&Type> {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Vec" {
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(GenericArgument::Type(inner)) = args.args.first() {
                        return Some(inner);
                    }
                }
            }
        }
    }
    None
}

/// Get the last segment name of a type path.
fn type_name(ty: &Type) -> Option<String> {
    if let Type::Path(type_path) = ty {
        type_path.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

/// Generate token stream for a type's JSON schema properties as a serde_json::Map.
fn type_to_schema_tokens(ty: &Type) -> TokenStream {
    // Check for Vec<T>
    if let Some(inner) = unwrap_vec(ty) {
        let items = type_to_schema_tokens(inner);
        return quote! {
            {
                let mut m = serde_json::Map::new();
                m.insert("type".to_string(), serde_json::Value::String("array".to_string()));
                m.insert("items".to_string(), serde_json::Value::Object(#items));
                m
            }
        };
    }

    let type_str = match type_name(ty).as_deref() {
        Some("String") | Some("str") => "string",
        Some("i8") | Some("i16") | Some("i32") | Some("i64") | Some("i128") | Some("isize") => {
            "integer"
        }
        Some("u8") | Some("u16") | Some("u32") | Some("u64") | Some("u128") | Some("usize") => {
            "integer"
        }
        Some("f32") | Some("f64") => "number",
        Some("bool") => "boolean",
        Some("Value") => {
            // serde_json::Value — any JSON
            return quote! {
                serde_json::Map::new()
            };
        }
        _ => "object",
    };

    quote! {
        {
            let mut m = serde_json::Map::new();
            m.insert("type".to_string(), serde_json::Value::String(#type_str.to_string()));
            m
        }
    }
}
