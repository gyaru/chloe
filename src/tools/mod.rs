pub mod tool_definitions;
pub mod tool_executor;

use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub parameters: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub id: String,
    pub success: bool,
    pub result: String,
    pub error: Option<String>,
}

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    async fn execute(&self, parameters: HashMap<String, Value>) -> Result<String, String>;
}