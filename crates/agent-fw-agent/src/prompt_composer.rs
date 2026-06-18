//! Pure prompt construction — structured data → Markdown string.
//!
//! # Design
//!
//! `PromptComposer` is a **value** that describes a prompt's section structure.
//! `render()` is a pure fold over sections — no I/O, no template engine, no
//! runtime errors. The rendered Markdown string is derived, never mutated.
//!
//! # Why Not a Template Engine?
//!
//! Template engines (Handlebars, Tera) introduce partial functions: a missing
//! variable is a runtime error, and composition is stringly-typed. Here each
//! [`PromptSection`] variant carries exactly the data it needs, making missing
//! data a compile-time error (make illegal states unrepresentable).
//!
//! # Integration
//!
//! The rendered string feeds into [`AgentBlueprint::new(model, prompt)`](crate::AgentBlueprint)
//! or [`SystemPrompt::new(prompt)`](crate::SystemPrompt). Schema content is
//! pre-rendered via `SchemaContext::to_prompt_summary()` (in `agent-fw-resolve`),
//! keeping this module free of cross-crate dependencies.
//!
//! # Algebraic Laws
//!
//! **L1 (Determinism)**: `render()` is a pure function. Same `PromptComposer`
//! value → byte-identical output. Tool definitions are sorted by name to
//! ensure stability even when sourced from unordered collections.
//!
//! **L2 (Monotonicity)**: Adding a non-empty section never reduces the output
//! length. `|composer.section(s).render()| >= |composer.render()|` when `s`
//! renders non-empty. Each section renders independently.
//!
//! **L3 (Tool Table Fidelity)**: The ToolTable section renders exactly the
//! definitions provided — no definitions are added, removed, or modified.
//! Output is sorted by tool name for deterministic ordering.
//!
//! **L4 (Section Independence)**: Each section's rendered output depends only
//! on its own data, never on the contents of other sections.

use crate::ToolDefinition;

// ─── PromptSection ──────────────────────────────────────────────────

/// A section of a system prompt.
///
/// Each variant carries exactly the data needed for rendering. No `Option`
/// fields that could be `None` at render time — this is a total function
/// from variant to Markdown.
///
/// Empty sections (empty Vec, empty String) are silently skipped during
/// rendering, so callers can unconditionally add sections with whatever
/// data is available.
#[derive(Debug, Clone)]
pub enum PromptSection {
    /// Agent identity and purpose.
    ///
    /// Renders as:
    /// ```text
    /// # Role
    /// You are the {name} — {purpose}.
    /// ```
    Role { name: String, purpose: String },

    /// Tool reference table derived from tool definitions.
    ///
    /// Renders as a Markdown table sorted by tool name (L1, L3).
    /// Pipe characters in descriptions are escaped; newlines collapsed.
    /// Empty Vec renders nothing.
    ToolTable(Vec<ToolDefinition>),

    /// Database schema summary (pre-rendered Markdown).
    ///
    /// Typically produced by `SchemaContext::to_prompt_summary()`.
    /// Renders under a `# Database Schema` heading.
    /// Empty string renders nothing.
    Schema(String),

    /// Behavioral decision tree — free-form Markdown guidance.
    ///
    /// Renders under `# Decision Tree`. Empty string renders nothing.
    DecisionTree(String),

    /// Ordered rules/constraints rendered as a bullet list.
    ///
    /// Maintains insertion order: rules are presented in the sequence
    /// given. Empty Vec renders nothing.
    Rules(Vec<String>),

    /// Extensible custom section with heading and body.
    ///
    /// Use for domain-specific sections (e.g., "Scope", "Knowledge",
    /// "Constraints"). Renders under `# {heading}`.
    Custom { heading: String, body: String },
}

impl PromptSection {
    /// `PromptSection::role("Coordinator", "orchestrating scenarios")`
    pub fn role(name: impl Into<String>, purpose: impl Into<String>) -> Self {
        Self::Role {
            name: name.into(),
            purpose: purpose.into(),
        }
    }

    /// `PromptSection::tool_table(dispatcher.tool_definitions())`
    pub fn tool_table(defs: Vec<ToolDefinition>) -> Self {
        Self::ToolTable(defs)
    }

    /// `PromptSection::schema(summary)`
    pub fn schema(summary: impl Into<String>) -> Self {
        Self::Schema(summary.into())
    }

    /// `PromptSection::decision_tree(tree)`
    pub fn decision_tree(tree: impl Into<String>) -> Self {
        Self::DecisionTree(tree.into())
    }

    /// `PromptSection::rules(["Never auto-execute", "Be concise"])`
    pub fn rules<I, S>(rules: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Rules(rules.into_iter().map(Into::into).collect())
    }

    /// `PromptSection::custom("Scope", "Price changes only.")`
    pub fn custom(heading: impl Into<String>, body: impl Into<String>) -> Self {
        Self::Custom {
            heading: heading.into(),
            body: body.into(),
        }
    }
}

// ─── PromptComposer ─────────────────────────────────────────────────

/// Pure prompt construction — structured data → Markdown string.
///
/// # Usage
///
/// ```ignore
/// use agent_fw_agent::{PromptComposer, PromptSection};
///
/// // Reads as a sentence — each method IS a section:
/// let prompt = PromptComposer::with_role("Coordinator", "orchestrating scenarios")
///     .tools(dispatcher.tool_definitions())
///     .schema(schema_summary)
///     .rules(["Never auto-execute plans", "Keep responses concise"])
///     .custom_section("Scope", "Price changes only.")
///     .render();
///
/// // Or build sections as values (programs-as-values):
/// let sections = vec![
///     PromptSection::role("Coordinator", "orchestrating"),
///     PromptSection::tool_table(tools),
///     PromptSection::rules(["Never skip approval"]),
/// ];
///
/// let blueprint = AgentBlueprint::new(model_id, prompt);
/// ```
#[derive(Debug, Clone)]
pub struct PromptComposer {
    sections: Vec<PromptSection>,
}

impl PromptComposer {
    /// Create an empty composer (identity for section composition).
    pub fn new() -> Self {
        Self {
            sections: Vec::new(),
        }
    }

    /// Convenience: start with a Role section.
    pub fn with_role(name: impl Into<String>, purpose: impl Into<String>) -> Self {
        Self::new().section(PromptSection::Role {
            name: name.into(),
            purpose: purpose.into(),
        })
    }

    /// Convenience: start with Role + ToolTable sections.
    ///
    /// Common pattern for agent prompts that need tool documentation.
    /// The tool table is derived from the definitions provided (typically
    /// via `dispatcher.tool_definitions()`).
    pub fn with_role_and_tools(
        name: impl Into<String>,
        purpose: impl Into<String>,
        tool_defs: Vec<ToolDefinition>,
    ) -> Self {
        Self::with_role(name, purpose).section(PromptSection::ToolTable(tool_defs))
    }

    /// Append a section (fluent builder).
    ///
    /// Sections render in the order they are added.
    pub fn section(mut self, s: PromptSection) -> Self {
        self.sections.push(s);
        self
    }

    // ── Convenience methods: each reads as a sentence ─────────────────

    /// Add a database schema section.
    ///
    /// Reads as: `composer.schema(summary)`.
    /// Empty string renders nothing (L2 monotonicity).
    pub fn schema(self, summary: impl Into<String>) -> Self {
        self.section(PromptSection::Schema(summary.into()))
    }

    /// Add a decision tree section.
    ///
    /// Reads as: `composer.decision_tree(tree)`.
    pub fn decision_tree(self, tree: impl Into<String>) -> Self {
        self.section(PromptSection::DecisionTree(tree.into()))
    }

    /// Add a rules section from any iterator of string-like items.
    ///
    /// Reads as: `composer.rules(["Never skip approval", "Be concise"])`.
    /// Also accepts `&[&str]`: `composer.rules(MY_RULES)`.
    pub fn rules<I, S>(self, rules: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.section(PromptSection::Rules(
            rules.into_iter().map(Into::into).collect(),
        ))
    }

    /// Add a custom named section.
    ///
    /// Reads as: `composer.custom_section("Scope", scope_text)`.
    pub fn custom_section(self, heading: impl Into<String>, body: impl Into<String>) -> Self {
        self.section(PromptSection::Custom {
            heading: heading.into(),
            body: body.into(),
        })
    }

    /// Add a tool table section from tool definitions.
    ///
    /// Reads as: `composer.tools(dispatcher.tool_definitions())`.
    pub fn tools(self, defs: Vec<ToolDefinition>) -> Self {
        self.section(PromptSection::ToolTable(defs))
    }

    /// Number of sections.
    pub fn len(&self) -> usize {
        self.sections.len()
    }

    /// Whether the composer has no sections.
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    /// Access the sections.
    pub fn sections(&self) -> &[PromptSection] {
        &self.sections
    }

    /// Render all sections to a Markdown string.
    ///
    /// Pure fold: `Vec<PromptSection> → String`. No I/O, no template
    /// resolution, no fallible operations. Empty sections are skipped.
    /// Non-empty sections are separated by blank lines.
    pub fn render(&self) -> String {
        let blocks: Vec<String> = self.sections.iter().filter_map(render_section).collect();
        blocks.join("\n")
    }
}

impl Default for PromptComposer {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Rendering (pure functions) ─────────────────────────────────────

/// Render a single section to a Markdown block.
///
/// Returns `None` for empty sections (empty tools, empty schema, etc.).
/// Each returned block ends with a single newline, so `join("\n")`
/// produces blank-line separators between sections.
fn render_section(section: &PromptSection) -> Option<String> {
    match section {
        PromptSection::Role { name, purpose } => {
            Some(format!("# Role\nYou are the {name} \u{2014} {purpose}.\n"))
        }

        PromptSection::ToolTable(defs) if defs.is_empty() => None,
        PromptSection::ToolTable(defs) => Some(render_tool_table(defs)),

        PromptSection::Schema(s) if s.is_empty() => None,
        PromptSection::Schema(summary) => {
            let mut out = String::from("# Database Schema\n");
            out.push_str(summary);
            ensure_trailing_newline(&mut out);
            Some(out)
        }

        PromptSection::DecisionTree(t) if t.is_empty() => None,
        PromptSection::DecisionTree(tree) => {
            let mut out = String::from("# Decision Tree\n");
            out.push_str(tree);
            ensure_trailing_newline(&mut out);
            Some(out)
        }

        PromptSection::Rules(rules) if rules.is_empty() => None,
        PromptSection::Rules(rules) => {
            let mut out = String::from("# Rules\n");
            for rule in rules {
                out.push_str("- ");
                out.push_str(rule);
                out.push('\n');
            }
            Some(out)
        }

        PromptSection::Custom { heading, body } => {
            let mut out = format!("# {heading}\n");
            out.push_str(body);
            ensure_trailing_newline(&mut out);
            Some(out)
        }
    }
}

/// Render a tool definitions table, sorted by name for determinism (L1, L3).
fn render_tool_table(defs: &[ToolDefinition]) -> String {
    let mut sorted: Vec<&ToolDefinition> = defs.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));

    let mut out = String::from("# Tools\n| Tool | Purpose |\n|------|---------|");
    for def in &sorted {
        let desc = sanitize_table_cell(&def.description);
        out.push_str(&format!("\n| `{}` | {} |", def.name, desc));
    }
    out.push('\n');
    out
}

/// Sanitize a string for inclusion in a Markdown table cell.
///
/// - Replaces `|` with `\|` to prevent column breaks
/// - Replaces newlines with spaces to keep the row single-line
fn sanitize_table_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

/// Ensure a string ends with exactly one newline.
fn ensure_trailing_newline(s: &mut String) {
    if !s.ends_with('\n') {
        s.push('\n');
    }
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_def(name: &str, desc: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.into(),
            description: desc.into(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    // ─── L1: Determinism ────────────────────────────────────────

    #[test]
    fn determinism_same_input_same_output() {
        let composer = PromptComposer::new()
            .section(PromptSection::Role {
                name: "Agent".into(),
                purpose: "helping users".into(),
            })
            .section(PromptSection::Rules(vec!["Be helpful".into()]));

        let a = composer.render();
        let b = composer.render();
        assert_eq!(a, b);
    }

    #[test]
    fn determinism_tool_table_sorted_regardless_of_input_order() {
        let tools_abc = vec![
            tool_def("alpha", "First"),
            tool_def("beta", "Second"),
            tool_def("gamma", "Third"),
        ];
        let tools_cba = vec![
            tool_def("gamma", "Third"),
            tool_def("beta", "Second"),
            tool_def("alpha", "First"),
        ];

        let a = PromptComposer::new()
            .section(PromptSection::ToolTable(tools_abc))
            .render();
        let b = PromptComposer::new()
            .section(PromptSection::ToolTable(tools_cba))
            .render();
        assert_eq!(a, b);
    }

    // ─── L2: Monotonicity ───────────────────────────────────────

    #[test]
    fn monotonicity_adding_section_grows_output() {
        let base = PromptComposer::new().section(PromptSection::Role {
            name: "Agent".into(),
            purpose: "test".into(),
        });
        let extended = base
            .clone()
            .section(PromptSection::Rules(vec!["Rule 1".into()]));

        assert!(extended.render().len() > base.render().len());
    }

    #[test]
    fn monotonicity_empty_sections_no_effect() {
        let base = PromptComposer::new().section(PromptSection::Role {
            name: "Agent".into(),
            purpose: "test".into(),
        });
        let base_output = base.render();

        // Each empty variant should produce identical output
        let with_empty_tools = PromptComposer::new()
            .section(PromptSection::Role {
                name: "Agent".into(),
                purpose: "test".into(),
            })
            .section(PromptSection::ToolTable(vec![]));
        let with_empty_schema = PromptComposer::new()
            .section(PromptSection::Role {
                name: "Agent".into(),
                purpose: "test".into(),
            })
            .section(PromptSection::Schema(String::new()));
        let with_empty_rules = PromptComposer::new()
            .section(PromptSection::Role {
                name: "Agent".into(),
                purpose: "test".into(),
            })
            .section(PromptSection::Rules(vec![]));
        let with_empty_tree = PromptComposer::new()
            .section(PromptSection::Role {
                name: "Agent".into(),
                purpose: "test".into(),
            })
            .section(PromptSection::DecisionTree(String::new()));

        assert_eq!(with_empty_tools.render(), base_output);
        assert_eq!(with_empty_schema.render(), base_output);
        assert_eq!(with_empty_rules.render(), base_output);
        assert_eq!(with_empty_tree.render(), base_output);
    }

    // ─── L3: Tool Table Fidelity ────────────────────────────────

    #[test]
    fn tool_table_contains_all_tools() {
        let tools = vec![
            tool_def("draft_plan", "Create a plan"),
            tool_def("approve_plan", "Execute a plan"),
            tool_def("query_data", "Resolve terms"),
        ];
        let output = PromptComposer::new()
            .section(PromptSection::ToolTable(tools))
            .render();

        assert!(output.contains("`draft_plan`"));
        assert!(output.contains("`approve_plan`"));
        assert!(output.contains("`query_data`"));
        assert!(output.contains("Create a plan"));
        assert!(output.contains("Execute a plan"));
        assert!(output.contains("Resolve terms"));
    }

    #[test]
    fn tool_table_sorted_by_name() {
        let tools = vec![
            tool_def("zebra", "Last"),
            tool_def("alpha", "First"),
            tool_def("middle", "Middle"),
        ];
        let output = PromptComposer::new()
            .section(PromptSection::ToolTable(tools))
            .render();

        let alpha_pos = output.find("`alpha`").unwrap();
        let middle_pos = output.find("`middle`").unwrap();
        let zebra_pos = output.find("`zebra`").unwrap();
        assert!(alpha_pos < middle_pos);
        assert!(middle_pos < zebra_pos);
    }

    // ─── L4: Section Independence ───────────────────────────────

    #[test]
    fn section_independence_role_unaffected_by_tools() {
        let role_block = "# Role\nYou are the Coordinator \u{2014} orchestrating.\n";

        let role_only = PromptComposer::new()
            .section(PromptSection::Role {
                name: "Coordinator".into(),
                purpose: "orchestrating".into(),
            })
            .render();
        let role_plus_tools = PromptComposer::new()
            .section(PromptSection::Role {
                name: "Coordinator".into(),
                purpose: "orchestrating".into(),
            })
            .section(PromptSection::ToolTable(vec![tool_def("t", "d")]))
            .render();

        assert!(role_only.starts_with(role_block));
        assert!(role_plus_tools.starts_with(role_block));
    }

    // ─── Rendering: each section type ───────────────────────────

    #[test]
    fn role_renders_with_em_dash() {
        let output = PromptComposer::new()
            .section(PromptSection::Role {
                name: "Planner".into(),
                purpose: "resolving products and scope".into(),
            })
            .render();
        assert!(output.contains("You are the Planner \u{2014} resolving products and scope."));
    }

    #[test]
    fn schema_renders_with_heading() {
        let output = PromptComposer::new()
            .section(PromptSection::Schema("## Products\n- col_a\n".into()))
            .render();
        assert!(output.starts_with("# Database Schema\n"));
        assert!(output.contains("## Products"));
        assert!(output.contains("- col_a"));
    }

    #[test]
    fn decision_tree_renders_content() {
        let tree = "When ambiguous \u{2192} query_data\nWhen clear \u{2192} draft_plan";
        let output = PromptComposer::new()
            .section(PromptSection::DecisionTree(tree.into()))
            .render();
        assert!(output.contains("# Decision Tree\n"));
        assert!(output.contains(tree));
    }

    #[test]
    fn rules_render_as_bullets() {
        let output = PromptComposer::new()
            .section(PromptSection::Rules(vec![
                "Never auto-execute".into(),
                "Keep responses concise".into(),
            ]))
            .render();
        assert!(output.contains("# Rules\n"));
        assert!(output.contains("- Never auto-execute\n"));
        assert!(output.contains("- Keep responses concise\n"));
    }

    #[test]
    fn custom_section_renders() {
        let output = PromptComposer::new()
            .section(PromptSection::Custom {
                heading: "Scope".into(),
                body: "Price changes only. Refuse other requests.\n".into(),
            })
            .render();
        assert!(output.contains("# Scope\n"));
        assert!(output.contains("Price changes only. Refuse other requests."));
    }

    // ─── Edge cases ─────────────────────────────────────────────

    #[test]
    fn empty_composer_renders_empty() {
        assert_eq!(PromptComposer::new().render(), "");
    }

    #[test]
    fn tool_description_pipe_escaped() {
        let tools = vec![tool_def("query", "Run SQL | read-only")];
        let output = PromptComposer::new()
            .section(PromptSection::ToolTable(tools))
            .render();
        assert!(output.contains(r"Run SQL \| read-only"));
    }

    #[test]
    fn tool_description_newline_collapsed() {
        let tools = vec![tool_def("query", "Run SQL.\nReturns JSON.")];
        let output = PromptComposer::new()
            .section(PromptSection::ToolTable(tools))
            .render();
        assert!(output.contains("Run SQL. Returns JSON."));
    }

    #[test]
    fn sanitize_table_cell_combined() {
        assert_eq!(sanitize_table_cell("a|b\nc"), r"a\|b c");
    }

    // ─── Convenience constructors ───────────────────────────────

    #[test]
    fn with_role_creates_single_section() {
        let c = PromptComposer::with_role("Agent", "testing");
        assert_eq!(c.len(), 1);
        let output = c.render();
        assert!(output.contains("You are the Agent"));
    }

    #[test]
    fn with_role_and_tools_creates_two_sections() {
        let c = PromptComposer::with_role_and_tools(
            "Coordinator",
            "orchestrating",
            vec![tool_def("draft_plan", "Create plan")],
        );
        assert_eq!(c.len(), 2);
        let output = c.render();
        assert!(output.contains("Coordinator"));
        assert!(output.contains("`draft_plan`"));
    }

    // ─── Section separation ─────────────────────────────────────

    #[test]
    fn sections_separated_by_blank_line() {
        let output = PromptComposer::new()
            .section(PromptSection::Role {
                name: "Agent".into(),
                purpose: "test".into(),
            })
            .section(PromptSection::Rules(vec!["Be helpful".into()]))
            .render();

        // Blank line = double newline between sections
        assert!(output.contains(".\n\n# Rules"));
    }

    // ─── Full composition (integration) ─────────────────────────

    #[test]
    fn full_prompt_composition() {
        let output = PromptComposer::with_role_and_tools(
            "Coordinator",
            "orchestrating price scenarios",
            vec![
                tool_def("draft_plan", "Create a pricing plan"),
                tool_def("approve_plan", "Execute an approved plan"),
            ],
        )
        .section(PromptSection::Schema(
            "## Products\n- display_name (varchar)\n".into(),
        ))
        .section(PromptSection::DecisionTree(
            "If ambiguous \u{2192} query_data first.\n".into(),
        ))
        .section(PromptSection::Rules(vec![
            "Pipeline: draft_plan \u{2192} approval \u{2192} approve_plan".into(),
            "Never auto-execute".into(),
        ]))
        .section(PromptSection::Custom {
            heading: "Scope".into(),
            body: "Price and availability changes only.\n".into(),
        })
        .render();

        // All sections present
        assert!(output.contains("# Role"));
        assert!(output.contains("# Tools"));
        assert!(output.contains("# Database Schema"));
        assert!(output.contains("# Decision Tree"));
        assert!(output.contains("# Rules"));
        assert!(output.contains("# Scope"));

        // Content correct
        assert!(output.contains("orchestrating price scenarios"));
        assert!(output.contains("`draft_plan`"));
        assert!(output.contains("`approve_plan`"));
        assert!(output.contains("display_name (varchar)"));
        assert!(output.contains("query_data first."));
        assert!(output.contains("- Never auto-execute"));
        assert!(output.contains("Price and availability changes only."));

        // Tools sorted
        let approve_pos = output.find("`approve_plan`").unwrap();
        let draft_pos = output.find("`draft_plan`").unwrap();
        assert!(approve_pos < draft_pos);
    }

    // ─── Convenience methods ───────────────────────────────────

    #[test]
    fn convenience_schema() {
        let output = PromptComposer::new()
            .schema("## Products\n- col_a\n")
            .render();
        assert!(output.contains("# Database Schema"));
        assert!(output.contains("col_a"));
    }

    #[test]
    fn convenience_decision_tree() {
        let output = PromptComposer::new()
            .decision_tree("If ambiguous → resolve")
            .render();
        assert!(output.contains("# Decision Tree"));
        assert!(output.contains("If ambiguous"));
    }

    #[test]
    fn convenience_rules_from_slice() {
        let rules: &[&str] = &["Rule A", "Rule B"];
        let output = PromptComposer::new().rules(rules.iter().copied()).render();
        assert!(output.contains("- Rule A"));
        assert!(output.contains("- Rule B"));
    }

    #[test]
    fn convenience_rules_from_vec() {
        let output = PromptComposer::new()
            .rules(vec!["Rule A".to_string(), "Rule B".to_string()])
            .render();
        assert!(output.contains("- Rule A"));
        assert!(output.contains("- Rule B"));
    }

    #[test]
    fn convenience_custom_section() {
        let output = PromptComposer::new()
            .custom_section("Scope", "Price changes only.")
            .render();
        assert!(output.contains("# Scope"));
        assert!(output.contains("Price changes only."));
    }

    #[test]
    fn convenience_tools() {
        let output = PromptComposer::new()
            .tools(vec![tool_def("draft_plan", "Create a plan")])
            .render();
        assert!(output.contains("`draft_plan`"));
    }

    #[test]
    fn convenience_methods_compose_fluently() {
        // This is the "reads as a sentence" test
        let output = PromptComposer::with_role("Coordinator", "orchestrating scenarios")
            .tools(vec![tool_def("draft_plan", "Create a plan")])
            .schema("## Products")
            .decision_tree("If ambiguous → resolve first")
            .rules(["Never skip approval", "Be concise"])
            .custom_section("Scope", "Price changes only.")
            .render();

        assert!(output.contains("Coordinator"));
        assert!(output.contains("`draft_plan`"));
        assert!(output.contains("# Database Schema"));
        assert!(output.contains("# Decision Tree"));
        assert!(output.contains("# Rules"));
        assert!(output.contains("# Scope"));
    }

    // ─── Struct properties ──────────────────────────────────────

    #[test]
    fn default_is_empty() {
        let c = PromptComposer::default();
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
        assert!(c.sections().is_empty());
    }

    #[test]
    fn sections_accessible() {
        let c = PromptComposer::new()
            .section(PromptSection::role("A", "B"))
            .section(PromptSection::rules(["R"]));
        assert_eq!(c.len(), 2);
        assert_eq!(c.sections().len(), 2);
    }

    // ─── PromptSection convenience constructors ─────────────────

    #[test]
    fn section_role_constructor() {
        let output = PromptComposer::new()
            .section(PromptSection::role("Agent", "helping"))
            .render();
        assert!(output.contains("You are the Agent"));
    }

    #[test]
    fn section_schema_constructor() {
        let output = PromptComposer::new()
            .section(PromptSection::schema("## Products\n- col_a\n"))
            .render();
        assert!(output.contains("# Database Schema"));
    }

    #[test]
    fn section_rules_constructor() {
        let output = PromptComposer::new()
            .section(PromptSection::rules(["Rule A", "Rule B"]))
            .render();
        assert!(output.contains("- Rule A"));
        assert!(output.contains("- Rule B"));
    }

    #[test]
    fn section_decision_tree_constructor() {
        let output = PromptComposer::new()
            .section(PromptSection::decision_tree("If ambiguous → resolve"))
            .render();
        assert!(output.contains("# Decision Tree"));
    }

    #[test]
    fn section_custom_constructor() {
        let output = PromptComposer::new()
            .section(PromptSection::custom("Scope", "Price changes only."))
            .render();
        assert!(output.contains("# Scope"));
    }

    #[test]
    fn section_tool_table_constructor() {
        let output = PromptComposer::new()
            .section(PromptSection::tool_table(vec![tool_def(
                "draft_plan",
                "Create plan",
            )]))
            .render();
        assert!(output.contains("`draft_plan`"));
    }

    #[test]
    fn sections_as_values_compose() {
        // Programs-as-values: build section list, then fold
        let sections = vec![
            PromptSection::role("Coordinator", "orchestrating"),
            PromptSection::rules(["Be concise"]),
            PromptSection::custom("Scope", "Price only."),
        ];
        let mut composer = PromptComposer::new();
        for s in sections {
            composer = composer.section(s);
        }
        let output = composer.render();
        assert!(output.contains("Coordinator"));
        assert!(output.contains("- Be concise"));
        assert!(output.contains("# Scope"));
    }
}
