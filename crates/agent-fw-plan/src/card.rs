//! Tagless final card algebra and interpreters.
//!
//! # CardAlg — The Algebra
//!
//! `CardAlg` defines a language for constructing UI cards. Programs
//! written against `A: CardAlg` are polymorphic — the same program
//! produces JSON, markdown, HTML, or any other format by choosing
//! a different interpreter.
//!
//! ## Law
//!
//! **L1 (Determinism)**: `render(card(t, d, attrs, actions))` produces
//! the same `String` for the same inputs, for any fixed interpreter.
//!
//! # Shipped Interpreters
//!
//! - [`PlainText`] — Markdown output
//! - [`JsonCard`] — CommandCard JSON (generic JSON-based card)
//! - [`JsonSweepCard`] — SweepCard JSON (typed stats + points + breakeven)

use serde_json::json;
use std::cmp::Ordering;

// ─── Domain Types ─────────────────────────────────────────────────────

/// A data point in a chart series.
#[derive(Clone, Debug)]
pub struct SeriesPoint {
    pub label: String,
    pub value: f64,
    pub highlight: Option<SeriesHighlight>,
}

/// Visual highlight marker for a series point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SeriesHighlight {
    Max,
    Min,
    Breakeven,
}

/// Button visual style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Danger,
}

/// Callout/info box visual style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalloutVariant {
    Info,
    Warning,
}

// ─── CardAlg Trait ────────────────────────────────────────────────────

/// Tagless final algebra for card construction.
///
/// Programs written generically against `A: CardAlg` are interpreted
/// by choosing a concrete type for `A`. This eliminates the need for
/// an intermediate AST — the program IS the data.
///
/// # Example
///
/// ```
/// use agent_fw_plan::card::*;
///
/// fn build_card<A: CardAlg>(alg: &A) -> A::Card {
///     let attrs = vec![
///         alg.stat_card("Count", "42"),
///         alg.detail_row("Status", "active"),
///     ];
///     alg.card("My Card", "Description", attrs, vec![])
/// }
///
/// // Same program, different outputs:
/// let json_output = JsonCard.render(build_card(&JsonCard));
/// let text_output = PlainText.render(build_card(&PlainText));
/// ```
pub trait CardAlg {
    /// Attribute/section representation.
    type Attr;
    /// Action button representation.
    type Action;
    /// Complete card representation.
    type Card;

    /// A metric stat card (displayed prominently).
    fn stat_card(&self, label: &str, value: &str) -> Self::Attr;

    /// A key-value detail row.
    fn detail_row(&self, label: &str, value: &str) -> Self::Attr;

    /// A collapsible section with summary + expandable detail.
    fn collapsible(&self, label: &str, summary: &str, detail: &str) -> Self::Attr;

    /// An informational or warning callout.
    fn callout(&self, variant: CalloutVariant, message: &str) -> Self::Attr;

    /// A data series (e.g., sweep points: label → percent_change).
    fn data_series(&self, label: &str, points: &[SeriesPoint]) -> Self::Attr;

    /// An action button.
    fn button(&self, id: &str, label: &str, variant: ButtonVariant) -> Self::Action;

    /// Assemble a complete card from metadata + attributes + actions.
    fn card(
        &self,
        title: &str,
        description: &str,
        attrs: Vec<Self::Attr>,
        actions: Vec<Self::Action>,
    ) -> Self::Card;

    /// Render the card to its final string representation.
    fn render(&self, card: Self::Card) -> String;
}

// ─── Interpreter: PlainText (Markdown) ────────────────────────────────

/// Markdown interpreter for CardAlg.
///
/// Produces human-readable text output. Buttons are suppressed.
pub struct PlainText;

impl CardAlg for PlainText {
    type Attr = String;
    type Action = String;
    type Card = String;

    fn stat_card(&self, label: &str, value: &str) -> Self::Attr {
        let first_line = value.lines().next().unwrap_or(value);
        format!("- {label}: {first_line}")
    }

    fn detail_row(&self, label: &str, value: &str) -> Self::Attr {
        format!("- {label}: {value}")
    }

    fn collapsible(&self, label: &str, summary: &str, _detail: &str) -> Self::Attr {
        format!("- {label}: {summary}")
    }

    fn callout(&self, _variant: CalloutVariant, message: &str) -> Self::Attr {
        format!("- Note: {message}")
    }

    fn data_series(&self, label: &str, points: &[SeriesPoint]) -> Self::Attr {
        if points.is_empty() {
            return format!("- {label}: No data");
        }

        let max_p = points
            .iter()
            .max_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(Ordering::Equal));
        let min_p = points
            .iter()
            .min_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(Ordering::Equal));

        match (min_p, max_p) {
            (Some(lo), Some(hi)) => format!(
                "- {label}: {} points, {:+.1}% ({}) to {:+.1}% ({})",
                points.len(),
                lo.value,
                lo.label,
                hi.value,
                hi.label,
            ),
            _ => format!("- {label}: {} points", points.len()),
        }
    }

    fn button(&self, _id: &str, _label: &str, _variant: ButtonVariant) -> Self::Action {
        String::new()
    }

    fn card(
        &self,
        title: &str,
        description: &str,
        attrs: Vec<Self::Attr>,
        _actions: Vec<Self::Action>,
    ) -> Self::Card {
        let mut lines = Vec::new();
        lines.push(format!("**{title}**"));
        if !description.is_empty() {
            lines.push(description.to_string());
        }
        for attr in attrs {
            if !attr.is_empty() {
                lines.push(attr);
            }
        }
        lines.join("\n")
    }

    fn render(&self, card: Self::Card) -> String {
        card
    }
}

// ─── Interpreter: JsonCard (CommandCard JSON) ─────────────────────────

/// Generic JSON card interpreter.
///
/// Produces a `CommandCard` component JSON structure. Stat cards
/// are placed in the "metrics" section; details in "default".
pub struct JsonCard;

impl CardAlg for JsonCard {
    type Attr = serde_json::Value;
    type Action = serde_json::Value;
    type Card = serde_json::Value;

    fn stat_card(&self, label: &str, value: &str) -> Self::Attr {
        json!({
            "label": label,
            "value": value,
            "section": "metrics",
            "cardStyle": "stat-card"
        })
    }

    fn detail_row(&self, label: &str, value: &str) -> Self::Attr {
        json!({
            "label": label,
            "value": value,
            "section": "default"
        })
    }

    fn collapsible(&self, label: &str, summary: &str, detail: &str) -> Self::Attr {
        json!({
            "label": label,
            "value": summary,
            "explanation": detail,
            "section": "context",
            "collapsible": true,
            "defaultExpanded": false
        })
    }

    fn callout(&self, variant: CalloutVariant, message: &str) -> Self::Attr {
        let prefix = match variant {
            CalloutVariant::Info => "\u{2139}\u{fe0f}",
            CalloutVariant::Warning => "\u{26a0}\u{fe0f}",
        };
        json!({
            "label": format!("{prefix} Note"),
            "value": message,
            "section": "context"
        })
    }

    fn data_series(&self, label: &str, points: &[SeriesPoint]) -> Self::Attr {
        if points.is_empty() {
            return json!({
                "label": label,
                "value": "No data",
                "section": "context"
            });
        }

        let mut lines = Vec::with_capacity(points.len());
        for p in points {
            let marker = match p.highlight {
                Some(SeriesHighlight::Max) => " \u{25b2} max",
                Some(SeriesHighlight::Min) => " \u{25bc} min",
                Some(SeriesHighlight::Breakeven) => " \u{25cf} breakeven",
                None => "",
            };
            lines.push(format!("{:<16} {:>+7.1}%{marker}", p.label, p.value));
        }

        let summary = format!("{} points", points.len());
        let detail = lines.join("\n");

        json!({
            "label": label,
            "value": summary,
            "explanation": detail,
            "section": "context",
            "collapsible": true,
            "defaultExpanded": true
        })
    }

    fn button(&self, id: &str, label: &str, variant: ButtonVariant) -> Self::Action {
        let v = match variant {
            ButtonVariant::Primary => "primary",
            ButtonVariant::Secondary => "secondary",
            ButtonVariant::Danger => "danger",
        };
        json!({ "id": id, "label": label, "variant": v })
    }

    fn card(
        &self,
        title: &str,
        description: &str,
        attrs: Vec<Self::Attr>,
        actions: Vec<Self::Action>,
    ) -> Self::Card {
        json!({
            "components": [{
                "name": "CommandCard",
                "props": {
                    "title": title,
                    "description": description,
                    "attributes": attrs,
                    "actions": actions
                }
            }]
        })
    }

    fn render(&self, card: Self::Card) -> String {
        serde_json::to_string(&card).unwrap_or_default()
    }
}

// ─── Interpreter: JsonSweepCard ───────────────────────────────────────

/// Typed attribute for sweep card partitioning.
///
/// Used internally by [`JsonSweepCard`] to partition card content
/// into typed sections (stats, points, callouts, details).
pub enum SweepAttr {
    Stat(serde_json::Value),
    Point(Vec<serde_json::Value>),
    Callout(serde_json::Value),
    Detail(serde_json::Value),
}

/// Sweep-specific JSON card interpreter.
///
/// Produces a `SweepCard` component with typed sections:
/// `stats`, `points`, `callouts`, `details`, `actions`.
pub struct JsonSweepCard;

impl CardAlg for JsonSweepCard {
    type Attr = SweepAttr;
    type Action = serde_json::Value;
    type Card = serde_json::Value;

    fn stat_card(&self, label: &str, value: &str) -> Self::Attr {
        SweepAttr::Stat(json!({ "label": label, "value": value }))
    }

    fn detail_row(&self, label: &str, value: &str) -> Self::Attr {
        SweepAttr::Detail(json!({
            "label": label,
            "summary": value,
            "content": value
        }))
    }

    fn collapsible(&self, label: &str, summary: &str, detail: &str) -> Self::Attr {
        SweepAttr::Detail(json!({
            "label": label,
            "summary": summary,
            "content": detail
        }))
    }

    fn callout(&self, variant: CalloutVariant, message: &str) -> Self::Attr {
        let variant_str = match variant {
            CalloutVariant::Info => "info",
            CalloutVariant::Warning => "warning",
        };
        let label = message
            .strip_prefix("Breakeven at ")
            .unwrap_or(message)
            .to_string();
        SweepAttr::Callout(json!({
            "variant": variant_str,
            "label": label,
            "message": message
        }))
    }

    fn data_series(&self, _label: &str, points: &[SeriesPoint]) -> Self::Attr {
        let pts: Vec<serde_json::Value> = points
            .iter()
            .map(|p| {
                let highlight = match &p.highlight {
                    Some(SeriesHighlight::Max) => json!("max"),
                    Some(SeriesHighlight::Min) => json!("min"),
                    Some(SeriesHighlight::Breakeven) => json!("breakeven"),
                    None => serde_json::Value::Null,
                };
                json!({
                    "label": p.label,
                    "value": p.value,
                    "highlight": highlight
                })
            })
            .collect();
        SweepAttr::Point(pts)
    }

    fn button(&self, id: &str, label: &str, variant: ButtonVariant) -> Self::Action {
        let v = match variant {
            ButtonVariant::Primary => "primary",
            ButtonVariant::Secondary => "secondary",
            ButtonVariant::Danger => "danger",
        };
        json!({ "id": id, "label": label, "variant": v })
    }

    fn card(
        &self,
        title: &str,
        description: &str,
        attrs: Vec<Self::Attr>,
        actions: Vec<Self::Action>,
    ) -> Self::Card {
        let mut stats = Vec::new();
        let mut points = Vec::new();
        let mut callouts = Vec::new();
        let mut details = Vec::new();

        for attr in attrs {
            match attr {
                SweepAttr::Stat(v) => stats.push(v),
                SweepAttr::Point(pts) => points = pts,
                SweepAttr::Callout(v) => callouts.push(v),
                SweepAttr::Detail(v) => details.push(v),
            }
        }

        // Extract breakeven callout for backward compat
        let breakeven = callouts
            .iter()
            .find(|c| {
                c["variant"] == "info"
                    && c["message"]
                        .as_str()
                        .is_some_and(|m| m.starts_with("Breakeven"))
            })
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let mut props = json!({
            "title": title,
            "points": points,
        });

        let obj = props.as_object_mut().unwrap();
        if !description.is_empty() {
            obj.insert("description".into(), json!(description));
        }
        if !stats.is_empty() {
            obj.insert("stats".into(), json!(stats));
        }
        if !breakeven.is_null() {
            obj.insert("breakeven".into(), breakeven);
        }
        if !callouts.is_empty() {
            obj.insert("callouts".into(), json!(callouts));
        }
        if !details.is_empty() {
            obj.insert("details".into(), json!(details));
        }
        if !actions.is_empty() {
            obj.insert("actions".into(), json!(actions));
        }

        json!({
            "components": [{
                "name": "SweepCard",
                "props": props
            }]
        })
    }

    fn render(&self, card: Self::Card) -> String {
        serde_json::to_string(&card).unwrap_or_default()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Polymorphic test program — same code, multiple interpreters.
    fn sample_program<A: CardAlg>(alg: &A) -> A::Card {
        let attrs = vec![
            alg.stat_card("Products", "42 products"),
            alg.stat_card("Metric", "revenue"),
            alg.data_series(
                "Revenue Impact",
                &[
                    SeriesPoint {
                        label: "-5%".into(),
                        value: -3.2,
                        highlight: Some(SeriesHighlight::Min),
                    },
                    SeriesPoint {
                        label: "0%".into(),
                        value: 0.0,
                        highlight: Some(SeriesHighlight::Breakeven),
                    },
                    SeriesPoint {
                        label: "+5%".into(),
                        value: 4.8,
                        highlight: Some(SeriesHighlight::Max),
                    },
                ],
            ),
            alg.callout(CalloutVariant::Info, "Breakeven at 0%"),
        ];
        let actions = vec![
            alg.button("apply", "Apply Best", ButtonVariant::Primary),
            alg.button("dismiss", "Dismiss", ButtonVariant::Secondary),
        ];
        alg.card(
            "Revenue Sweep: 3 points",
            "Test description",
            attrs,
            actions,
        )
    }

    // ─── PlainText Tests ──────────────────────────────────────────

    #[test]
    fn plaintext_produces_markdown() {
        let output = PlainText.render(sample_program(&PlainText));
        assert!(output.contains("**Revenue Sweep: 3 points**"));
        assert!(output.contains("Test description"));
        assert!(output.contains("Products: 42 products"));
        assert!(output.contains("3 points"));
    }

    #[test]
    fn plaintext_empty_data_series() {
        let attr = PlainText.data_series("Empty", &[]);
        assert_eq!(attr, "- Empty: No data");
    }

    // ─── JsonCard Tests ───────────────────────────────────────────

    #[test]
    fn json_card_produces_valid_structure() {
        let card = sample_program(&JsonCard);
        let output = JsonCard.render(card.clone());
        assert!(!output.is_empty());

        let components = card["components"].as_array().unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0]["name"], "CommandCard");

        let props = &components[0]["props"];
        assert_eq!(props["title"], "Revenue Sweep: 3 points");

        let attrs = props["attributes"].as_array().unwrap();
        assert!(attrs.len() >= 3);

        let actions = props["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0]["id"], "apply");
    }

    #[test]
    fn json_card_empty_data_series() {
        let attr = JsonCard.data_series("Empty", &[]);
        assert_eq!(attr["value"], "No data");
    }

    // ─── JsonSweepCard Tests ──────────────────────────────────────

    #[test]
    fn sweep_card_produces_typed_structure() {
        let card = sample_program(&JsonSweepCard);
        let output = JsonSweepCard.render(card.clone());
        assert!(!output.is_empty());

        let components = card["components"].as_array().unwrap();
        assert_eq!(components[0]["name"], "SweepCard");

        let props = &components[0]["props"];
        assert_eq!(props["title"], "Revenue Sweep: 3 points");

        // Stats are in typed array
        let stats = props["stats"].as_array().unwrap();
        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0]["label"], "Products");

        // Points are in typed array with numeric values
        let points = props["points"].as_array().unwrap();
        assert_eq!(points.len(), 3);
        assert!(points[0]["value"].is_f64());
        assert_eq!(points[0]["highlight"], "min");
        assert_eq!(points[2]["highlight"], "max");

        // Breakeven is a typed object
        assert!(props["breakeven"].is_object());
    }

    #[test]
    fn sweep_card_omits_empty_fields() {
        let card = JsonSweepCard.card("Title", "", vec![], vec![]);
        let props = &card["components"][0]["props"];
        assert!(props.get("description").is_none());
        assert!(props.get("stats").is_none());
        assert!(props.get("details").is_none());
        assert!(props.get("actions").is_none());
    }

    #[test]
    fn sweep_card_zero_products_warning() {
        let attrs = vec![JsonSweepCard.callout(CalloutVariant::Warning, "No products matched")];
        let card = JsonSweepCard.card("Sweep", "desc", attrs, vec![]);
        let callouts = card["components"][0]["props"]["callouts"]
            .as_array()
            .unwrap();
        assert_eq!(callouts[0]["variant"], "warning");
    }

    // ─── Determinism Law ──────────────────────────────────────────

    #[test]
    fn render_determinism_plaintext() {
        let a = PlainText.render(sample_program(&PlainText));
        let b = PlainText.render(sample_program(&PlainText));
        assert_eq!(a, b);
    }

    #[test]
    fn render_determinism_json() {
        let a = JsonCard.render(sample_program(&JsonCard));
        let b = JsonCard.render(sample_program(&JsonCard));
        assert_eq!(a, b);
    }

    #[test]
    fn render_determinism_sweep() {
        let a = JsonSweepCard.render(sample_program(&JsonSweepCard));
        let b = JsonSweepCard.render(sample_program(&JsonSweepCard));
        assert_eq!(a, b);
    }

    // ─── Empty Card Tests ─────────────────────────────────────────

    #[test]
    fn empty_card_renders_both() {
        let json = JsonCard.render(JsonCard.card("T", "D", vec![], vec![]));
        assert!(!json.is_empty());

        let text = PlainText.render(PlainText.card("T", "D", vec![], vec![]));
        assert!(text.contains("**T**"));
        assert!(text.contains("D"));
    }

    // ─── CountingInterpreter ──────────────────────────────────────────

    /// Attribute kind tag for counting.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum AttrKind {
        Stat,
        Detail,
        Collapsible,
        Callout,
        Series(usize), // point count
    }

    /// Algebraic counts extracted from a program without rendering.
    ///
    /// This is the "counting natural transformation": given the same program,
    /// any conforming interpreter must produce output consistent with these counts.
    #[derive(Debug, PartialEq, Eq)]
    struct CardCounts {
        title: String,
        stat_count: usize,
        detail_count: usize,
        callout_count: usize,
        series_point_count: usize,
        action_count: usize,
    }

    /// Interpreter that counts structural elements without rendering.
    ///
    /// This is the "free interpreter" — it observes the algebra calls
    /// and records their structure, making no rendering decisions.
    struct CountingInterpreter;

    impl CardAlg for CountingInterpreter {
        type Attr = AttrKind;
        type Action = ();
        type Card = CardCounts;

        fn stat_card(&self, _label: &str, _value: &str) -> Self::Attr {
            AttrKind::Stat
        }
        fn detail_row(&self, _label: &str, _value: &str) -> Self::Attr {
            AttrKind::Detail
        }
        fn collapsible(&self, _label: &str, _summary: &str, _detail: &str) -> Self::Attr {
            AttrKind::Collapsible
        }
        fn callout(&self, _variant: CalloutVariant, _message: &str) -> Self::Attr {
            AttrKind::Callout
        }
        fn data_series(&self, _label: &str, points: &[SeriesPoint]) -> Self::Attr {
            AttrKind::Series(points.len())
        }
        fn button(&self, _id: &str, _label: &str, _variant: ButtonVariant) -> Self::Action {}
        fn card(
            &self,
            title: &str,
            _description: &str,
            attrs: Vec<Self::Attr>,
            actions: Vec<Self::Action>,
        ) -> Self::Card {
            let stat_count = attrs.iter().filter(|a| matches!(a, AttrKind::Stat)).count();
            let detail_count = attrs
                .iter()
                .filter(|a| matches!(a, AttrKind::Detail | AttrKind::Collapsible))
                .count();
            let callout_count = attrs
                .iter()
                .filter(|a| matches!(a, AttrKind::Callout))
                .count();
            let series_point_count: usize = attrs
                .iter()
                .filter_map(|a| match a {
                    AttrKind::Series(n) => Some(n),
                    _ => None,
                })
                .sum();
            CardCounts {
                title: title.to_string(),
                stat_count,
                detail_count,
                callout_count,
                series_point_count,
                action_count: actions.len(),
            }
        }
        fn render(&self, card: Self::Card) -> String {
            format!("{card:?}")
        }
    }

    /// Same `sample_program` through JsonCard must produce output consistent
    /// with the CountingInterpreter's structural tallies across ALL 5 dimensions.
    ///
    /// This is a natural transformation check: the counting interpreter
    /// provides ground truth; JsonCard must agree on every dimension.
    /// PlainText is verified for title only (it deliberately suppresses actions
    /// and merges series into single lines — rendering decisions, not bugs).
    #[test]
    fn cross_interpreter_counting_consistency() {
        let counts = sample_program(&CountingInterpreter);

        // Ground truth from the algebra
        assert_eq!(counts.title, "Revenue Sweep: 3 points");
        assert_eq!(counts.stat_count, 2, "2 stat_card calls");
        assert_eq!(counts.detail_count, 0, "0 detail/collapsible calls");
        assert_eq!(counts.callout_count, 1, "1 callout call");
        assert_eq!(counts.series_point_count, 3, "3 series points");
        assert_eq!(counts.action_count, 2, "2 button calls");

        // Verify JsonCard agrees on ALL structural counts
        let json_card = sample_program(&JsonCard);
        let props = &json_card["components"][0]["props"];
        let attrs = props["attributes"].as_array().unwrap();

        // Title
        assert_eq!(
            props["title"].as_str().unwrap(),
            counts.title,
            "JsonCard title mismatch"
        );

        // Stat count: attributes with cardStyle == "stat-card"
        let json_stat_count = attrs
            .iter()
            .filter(|a| a.get("cardStyle").and_then(|v| v.as_str()) == Some("stat-card"))
            .count();
        assert_eq!(json_stat_count, counts.stat_count, "JsonCard stat count");

        // Action count
        let json_action_count = props["actions"].as_array().map(|a| a.len()).unwrap_or(0);
        assert_eq!(
            json_action_count, counts.action_count,
            "JsonCard action count"
        );

        // Total attribute count must equal stat + detail + callout + series
        let expected_attr_count =
            counts.stat_count + counts.detail_count + counts.callout_count + 1; // 1 series attr
        assert_eq!(
            attrs.len(),
            expected_attr_count,
            "JsonCard total attr count"
        );

        // Verify PlainText title (rendering decisions are PlainText's prerogative)
        let text = PlainText.render(sample_program(&PlainText));
        assert!(
            text.starts_with(&format!("**{}**", counts.title)),
            "PlainText title mismatch: {text}"
        );
    }
}
