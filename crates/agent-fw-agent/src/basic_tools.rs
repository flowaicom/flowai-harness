//! Stock basic Rig tools.
//!
//! These are intentionally small, generic tools that are useful in many
//! agents and especially in plain-chat fallback interpreters.

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A simple tool that returns the current time.
#[derive(Debug, Clone, Default)]
pub struct GetCurrentTimeTool;

#[derive(Debug, Deserialize)]
pub struct GetCurrentTimeArgs {
    /// Optional timezone label. The stock tool reports UTC time and echoes the
    /// requested label for caller context.
    #[serde(default)]
    pub timezone: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GetCurrentTimeOutput {
    pub time: String,
    pub timezone: String,
}

#[derive(Debug, Error)]
#[error("Time tool error: {0}")]
pub struct TimeToolError(String);

impl Tool for GetCurrentTimeTool {
    const NAME: &'static str = "get_current_time";

    type Error = TimeToolError;
    type Args = GetCurrentTimeArgs;
    type Output = GetCurrentTimeOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Get the current date and time. Useful when the user asks about the current time or date.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "timezone": {
                        "type": "string",
                        "description": "Timezone label to use (e.g., 'UTC', 'America/New_York'). Defaults to UTC."
                    }
                },
                "required": []
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let timezone = args.timezone.unwrap_or_else(|| "UTC".to_string());
        let now = chrono::Utc::now();

        Ok(GetCurrentTimeOutput {
            time: now.to_rfc3339(),
            timezone,
        })
    }
}

/// A simple calculator tool for basic arithmetic.
#[derive(Debug, Clone, Default)]
pub struct CalculatorTool;

#[derive(Debug, Deserialize)]
pub struct CalculatorArgs {
    pub expression: String,
}

#[derive(Debug, Serialize)]
pub struct CalculatorOutput {
    pub result: f64,
    pub expression: String,
}

#[derive(Debug, Error)]
#[error("Calculator error: {0}")]
pub struct CalculatorError(String);

impl Tool for CalculatorTool {
    const NAME: &'static str = "calculator";

    type Error = CalculatorError;
    type Args = CalculatorArgs;
    type Output = CalculatorOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Perform basic arithmetic calculations. Supports +, -, *, / operations."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "A simple arithmetic expression (e.g., '2 + 2', '10 * 5', '100 / 4')"
                    }
                },
                "required": ["expression"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let result = evaluate_simple_expression(&args.expression).map_err(CalculatorError)?;

        Ok(CalculatorOutput {
            result,
            expression: args.expression,
        })
    }
}

fn evaluate_simple_expression(expr: &str) -> Result<f64, String> {
    let expr = expr.trim();

    for op in ['+', '-', '*', '/'] {
        if let Some(pos) = expr.rfind(op) {
            if pos > 0 {
                let left = expr[..pos].trim();
                let right = expr[pos + 1..].trim();

                let left_val: f64 = left
                    .parse()
                    .map_err(|_| format!("Invalid number: {}", left))?;
                let right_val: f64 = right
                    .parse()
                    .map_err(|_| format!("Invalid number: {}", right))?;

                return match op {
                    '+' => Ok(left_val + right_val),
                    '-' => Ok(left_val - right_val),
                    '*' => Ok(left_val * right_val),
                    '/' => {
                        if right_val == 0.0 {
                            Err("Division by zero".to_string())
                        } else {
                            Ok(left_val / right_val)
                        }
                    }
                    other => Err(format!("Unsupported operator: {}", other)),
                };
            }
        }
    }

    expr.parse()
        .map_err(|_| format!("Could not parse expression: {}", expr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluate_addition() {
        assert_eq!(evaluate_simple_expression("2 + 3").unwrap(), 5.0);
    }

    #[test]
    fn evaluate_subtraction() {
        assert_eq!(evaluate_simple_expression("10 - 4").unwrap(), 6.0);
    }

    #[test]
    fn evaluate_multiplication() {
        assert_eq!(evaluate_simple_expression("3 * 4").unwrap(), 12.0);
    }

    #[test]
    fn evaluate_division() {
        assert_eq!(evaluate_simple_expression("15 / 3").unwrap(), 5.0);
    }

    #[test]
    fn evaluate_division_by_zero() {
        assert!(evaluate_simple_expression("10 / 0").is_err());
    }

    #[test]
    fn evaluate_single_number() {
        assert_eq!(evaluate_simple_expression("42").unwrap(), 42.0);
    }

    #[tokio::test]
    async fn calculator_tool_executes() {
        let tool = CalculatorTool;
        let result = tool
            .call(CalculatorArgs {
                expression: "2 + 2".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(result.result, 4.0);
    }

    #[tokio::test]
    async fn time_tool_executes() {
        let tool = GetCurrentTimeTool;
        let result = tool
            .call(GetCurrentTimeArgs {
                timezone: Some("UTC".to_string()),
            })
            .await
            .unwrap();
        assert!(!result.time.is_empty());
        assert_eq!(result.timezone, "UTC");
    }
}
