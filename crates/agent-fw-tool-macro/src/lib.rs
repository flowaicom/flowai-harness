//! Proc macros: `#[derive(ToolSchema)]` and `#[tool_handler]`
//!
//! # `#[derive(ToolSchema)]`
//!
//! Generates a `ToolSchema` implementation that produces a JSON schema.
//!
//! ```ignore
//! #[derive(Deserialize, ToolSchema)]
//! #[serde(rename_all = "camelCase")]
//! pub struct SearchInput {
//!     /// Search query text
//!     #[schema(required)]
//!     pub query: String,
//!
//!     /// Max results to return
//!     pub limit: Option<usize>,
//!
//!     /// Filter by category
//!     #[schema(enum_values = ["electronics", "clothing", "food"])]
//!     pub category: Option<String>,
//! }
//! ```
//!
//! # `#[tool_handler]`
//!
//! Generates a `ToolHandler` trait impl from a typed handle method.
//! Eliminates boilerplate: definition(), input deserialization, error wrapping.
//!
//! ```ignore
//! #[tool_handler(name = "draft_plan", description = "Create a plan", schema = BuildPlanSchema)]
//! impl BuildPlanHandler {
//!     async fn handle(&self, env: &ToolEnvironment, input: MyInput) -> Result<Value, ToolError> {
//!         // domain logic only — no ToolCallResult, no tool_use_id, no serde_json::from_value
//!     }
//! }
//! ```

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput, ItemImpl};

mod tool_handler;
mod tool_schema;

/// Derive macro for `ToolSchema`.
///
/// Generates a `json_schema()` method returning a JSON schema object.
///
/// # Supported Types
///
/// - `String` → `"string"`
/// - `i32`, `i64`, `u32`, `u64`, `usize`, `isize` → `"integer"`
/// - `f32`, `f64` → `"number"`
/// - `bool` → `"boolean"`
/// - `Vec<T>` → `{"type": "array", "items": <T>}`
/// - `Option<T>` → field is NOT required (unless `#[schema(required)]`)
/// - `serde_json::Value` → `{}`
///
/// # Attributes
///
/// - `#[schema(required)]` — forces required even if `Option`
/// - `#[schema(description = "...")]` — overrides doc comment
/// - `#[schema(enum_values = ["a", "b"])]` — adds `"enum"` constraint
/// - `#[serde(rename_all = "camelCase")]` — respects serde rename
/// - `#[serde(rename = "newName")]` — respects serde field rename
#[proc_macro_derive(ToolSchema, attributes(schema, serde))]
pub fn derive_tool_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    tool_schema::expand(input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

/// Attribute macro that generates a `ToolHandler` trait implementation.
///
/// Applied to an `impl` block containing a typed `handle` method.
/// Generates: `definition()` (from schema type), outer `handle()`
/// (deserialization + error wrapping). The user writes only domain logic.
///
/// # Attributes
///
/// - `name = "toolName"` — tool name for LLM registration (required)
/// - `description = "..."` — human-readable description (required)
/// - `schema = SchemaType` — type implementing `ToolSchema` for JSON schema (required)
///
/// # Generated Behavior
///
/// - `definition()`: returns `ToolDefinition` with name, description, and
///   `SchemaType::json_schema()` — pure, deterministic (L2)
/// - `handle(tool_use_id, Value, env)`: deserializes input via `serde_json::from_value`,
///   calls the user's `handle(env, typed_input)`, wraps `Ok` in
///   `ToolCallResult::success` and `Err` in `ToolCallResult::error` (L3: error totality)
///
/// # Example
///
/// ```ignore
/// #[tool_handler(name = "draft_plan", description = "Create a plan", schema = BuildPlanSchema)]
/// impl BuildPlanHandler {
///     async fn handle(
///         &self,
///         env: &ToolEnvironment,
///         input: BuildPlanInput<PricingAction>,
///     ) -> Result<Value, ToolError> {
///         let db = env.try_ext::<dyn TargetDatabase>()?;
///         // ... domain logic ...
///         Ok(serde_json::to_value(&output).unwrap())
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn tool_handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attrs = parse_macro_input!(attr as tool_handler::ToolHandlerAttrs);
    let item = parse_macro_input!(item as ItemImpl);
    tool_handler::expand(attrs, item)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}
