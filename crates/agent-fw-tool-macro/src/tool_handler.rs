//! Implementation of `#[tool_handler]`.
//!
//! Generates a `ToolHandler` trait implementation from an impl block
//! containing a `handle` method. The user writes typed domain logic;
//! the macro generates deserialization, error wrapping, and definition().
//!
//! # Generated Code
//!
//! Given:
//! ```ignore
//! #[tool_handler(name = "draft_plan", description = "Create a plan", schema = BuildPlanSchema)]
//! impl BuildPlanHandler {
//!     async fn handle(&self, env: &ToolEnvironment, input: MyInput) -> Result<Value, ToolError> {
//!         // domain logic
//!     }
//! }
//! ```
//!
//! The macro generates:
//! ```ignore
//! impl BuildPlanHandler {
//!     async fn handle(&self, env: &ToolEnvironment, input: MyInput) -> Result<Value, ToolError> {
//!         // domain logic (preserved as-is)
//!     }
//! }
//!
//! #[async_trait::async_trait]
//! impl agent_fw_agent::ToolHandler for BuildPlanHandler {
//!     fn definition(&self) -> agent_fw_agent::ToolDefinition {
//!         agent_fw_agent::ToolDefinition {
//!             name: "draft_plan".into(),
//!             description: "Create a plan".into(),
//!             input_schema: <BuildPlanSchema as agent_fw_tool::ToolSchema>::json_schema(),
//!         }
//!     }
//!
//!     async fn handle(
//!         &self,
//!         tool_use_id: &str,
//!         input: serde_json::Value,
//!         env: &agent_fw_tool::ToolEnvironment,
//!     ) -> agent_fw_agent::ToolCallResult {
//!         let typed_input = match serde_json::from_value(input) {
//!             Ok(v) => v,
//!             Err(e) => return agent_fw_agent::ToolCallResult::error(
//!                 tool_use_id, format!("Invalid input: {e}")
//!             ),
//!         };
//!         match Self::handle(self, env, typed_input).await {
//!             Ok(value) => agent_fw_agent::ToolCallResult::success(tool_use_id, value),
//!             Err(e) => agent_fw_agent::ToolCallResult::error(tool_use_id, e.to_string()),
//!         }
//!     }
//! }
//! ```

use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse::Parse, parse::ParseStream, Ident, ItemImpl, LitStr, Token, Type};

/// Parsed attributes from `#[tool_handler(name = "...", description = "...", schema = Type)]`.
pub struct ToolHandlerAttrs {
    pub name: LitStr,
    pub description: LitStr,
    pub schema: Type,
}

impl Parse for ToolHandlerAttrs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name: Option<LitStr> = None;
        let mut description: Option<LitStr> = None;
        let mut schema: Option<Type> = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match key.to_string().as_str() {
                "name" => {
                    name = Some(input.parse()?);
                }
                "description" => {
                    description = Some(input.parse()?);
                }
                "schema" => {
                    schema = Some(input.parse()?);
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown attribute: `{other}`. Expected: name, description, schema"
                        ),
                    ));
                }
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(ToolHandlerAttrs {
            name: name.ok_or_else(|| input.error("missing `name` attribute"))?,
            description: description
                .ok_or_else(|| input.error("missing `description` attribute"))?,
            schema: schema.ok_or_else(|| input.error("missing `schema` attribute"))?,
        })
    }
}

/// Expand the `#[tool_handler]` attribute macro.
pub fn expand(attrs: ToolHandlerAttrs, item: ItemImpl) -> syn::Result<TokenStream> {
    // Extract the self type (e.g., BuildPlanHandler)
    let self_ty = &item.self_ty;

    // Find the `handle` method and extract the input type
    let handle_method = item
        .items
        .iter()
        .find_map(|item| {
            if let syn::ImplItem::Fn(method) = item {
                if method.sig.ident == "handle" {
                    return Some(method);
                }
            }
            None
        })
        .ok_or_else(|| {
            syn::Error::new_spanned(
                &item,
                "#[tool_handler] requires a `handle` method in the impl block",
            )
        })?;

    // Check for optional `extension_manifest` method — forward it if present
    let has_extension_manifest = item.items.iter().any(|item| {
        if let syn::ImplItem::Fn(method) = item {
            method.sig.ident == "extension_manifest"
        } else {
            false
        }
    });

    // Extract the typed input parameter (third param: &self, env, input)
    let input_type = extract_input_type(&handle_method.sig)?;

    let tool_name = &attrs.name;
    let tool_desc = &attrs.description;
    let schema_type = &attrs.schema;

    // Conditionally generate extension_manifest forwarding
    let extension_manifest_impl = if has_extension_manifest {
        quote! {
            fn extension_manifest(&self) -> agent_fw_tool::ToolExtensionManifest {
                Self::extension_manifest(self)
            }
        }
    } else {
        quote! {}
    };

    Ok(quote! {
        // Preserve the user's impl block exactly as written
        #item

        // Generate ToolHandler trait impl
        #[async_trait::async_trait]
        impl agent_fw_agent::ToolHandler for #self_ty {
            fn definition(&self) -> agent_fw_agent::ToolDefinition {
                agent_fw_agent::ToolDefinition {
                    name: #tool_name.into(),
                    description: #tool_desc.into(),
                    input_schema: <#schema_type as agent_fw_tool::ToolSchema>::json_schema(),
                }
            }

            #extension_manifest_impl

            async fn handle(
                &self,
                tool_use_id: &str,
                input: serde_json::Value,
                env: &agent_fw_tool::ToolEnvironment,
            ) -> agent_fw_agent::ToolCallResult {
                let typed_input: #input_type = match serde_json::from_value(input) {
                    Ok(v) => v,
                    Err(e) => {
                        return agent_fw_agent::ToolCallResult::error(
                            tool_use_id,
                            format!("Invalid input: {e}"),
                        )
                    }
                };
                match Self::handle(self, env, typed_input).await {
                    Ok(mut value) => {
                        // Extract typed UI channels from the returned value.
                        // This bridges handlers that embed approvalDsl/displaySummary
                        // in their JSON to the typed ToolCallResult fields.
                        let mut approval_dsl = None;
                        let mut display_summary = None;
                        if let Some(obj) = value.as_object_mut() {
                            if let Some(v) = obj.remove("approvalDsl") {
                                approval_dsl = v.as_str().map(String::from)
                                    .or_else(|| serde_json::to_string(&v).ok());
                            }
                            if let Some(v) = obj.remove("displaySummary") {
                                display_summary = v.as_str().map(String::from);
                            }
                            obj.remove("_cardEmitted");
                        }
                        let mut result = agent_fw_agent::ToolCallResult::success(tool_use_id, value);
                        result.approval_dsl = approval_dsl;
                        result.display_summary = display_summary;
                        result
                    }
                    Err(e) => {
                        agent_fw_agent::ToolCallResult::error(tool_use_id, e.to_string())
                    }
                }
            }
        }
    })
}

/// Extract the input type from the third parameter of `handle(&self, env, input: T)`.
fn extract_input_type(sig: &syn::Signature) -> syn::Result<Type> {
    let params: Vec<_> = sig.inputs.iter().collect();

    // Expect: &self, env: &ToolEnvironment, input: T
    if params.len() < 3 {
        return Err(syn::Error::new_spanned(
            sig,
            "handle method must have signature: \
             async fn handle(&self, env: &ToolEnvironment, input: T) -> Result<Value, ToolError>",
        ));
    }

    match &params[2] {
        syn::FnArg::Typed(pat_type) => Ok((*pat_type.ty).clone()),
        _ => Err(syn::Error::new_spanned(
            &params[2],
            "third parameter must be a typed input (not self)",
        )),
    }
}
