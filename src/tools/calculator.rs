use super::Tool;
use serde_json::{Value, json};
use std::collections::HashMap;

pub struct CalculatorTool;

#[async_trait::async_trait]
impl Tool for CalculatorTool {
    fn name(&self) -> &str {
        "calculate"
    }

    fn description(&self) -> &str {
        "Perform mathematical calculations. Supports basic arithmetic operations."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "The mathematical expression to evaluate (e.g., '2 + 2', '10 * 5')"
                }
            },
            "required": ["expression"]
        })
    }

    async fn execute(
        &self,
        parameters: HashMap<String, Value>,
        _discord_context: Option<&super::DiscordContext>,
    ) -> Result<String, String> {
        let expression = parameters
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'expression' parameter")?;

        // Simple calculator - in a real implementation you'd use a proper math parser
        match expression.trim() {
            expr if expr.contains(" + ") => {
                let parts: Vec<&str> = expr.split(" + ").collect();
                if parts.len() == 2 {
                    let a: f64 = parts[0].parse().map_err(|_| "Invalid number")?;
                    let b: f64 = parts[1].parse().map_err(|_| "Invalid number")?;
                    Ok(format!("{} + {} = {}", a, b, a + b))
                } else {
                    Err("Invalid addition expression".to_string())
                }
            }
            expr if expr.contains(" - ") => {
                let parts: Vec<&str> = expr.split(" - ").collect();
                if parts.len() == 2 {
                    let a: f64 = parts[0].parse().map_err(|_| "Invalid number")?;
                    let b: f64 = parts[1].parse().map_err(|_| "Invalid number")?;
                    Ok(format!("{} - {} = {}", a, b, a - b))
                } else {
                    Err("Invalid subtraction expression".to_string())
                }
            }
            expr if expr.contains(" * ") => {
                let parts: Vec<&str> = expr.split(" * ").collect();
                if parts.len() == 2 {
                    let a: f64 = parts[0].parse().map_err(|_| "Invalid number")?;
                    let b: f64 = parts[1].parse().map_err(|_| "Invalid number")?;
                    Ok(format!("{} * {} = {}", a, b, a * b))
                } else {
                    Err("Invalid multiplication expression".to_string())
                }
            }
            expr if expr.contains(" / ") => {
                let parts: Vec<&str> = expr.split(" / ").collect();
                if parts.len() == 2 {
                    let a: f64 = parts[0].parse().map_err(|_| "Invalid number")?;
                    let b: f64 = parts[1].parse().map_err(|_| "Invalid number")?;
                    if b == 0.0 {
                        Err("Division by zero".to_string())
                    } else {
                        Ok(format!("{} / {} = {}", a, b, a / b))
                    }
                } else {
                    Err("Invalid division expression".to_string())
                }
            }
            _ => Err("Unsupported expression. Use format like '2 + 2', '10 * 5', etc.".to_string()),
        }
    }
}
