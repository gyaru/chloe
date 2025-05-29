use super::{Tool, ToolCall, ToolResult};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info};

pub struct ToolExecutor {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolExecutor {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register_tool(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    pub fn get_tool_definitions(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "parameters": tool.parameters_schema()
                })
            })
            .collect()
    }

    pub async fn execute_tool(&self, tool_call: ToolCall) -> ToolResult {
        info!(
            event = "tool_execution_start",
            tool_name = %tool_call.name,
            tool_id = %tool_call.id,
            parameters = ?tool_call.parameters,
            "Starting tool execution"
        );

        let result = match self.tools.get(&tool_call.name) {
            Some(tool) => {
                match tool.execute(tool_call.parameters).await {
                    Ok(result) => {
                        info!(
                            event = "tool_execution_success",
                            tool_name = %tool_call.name,
                            tool_id = %tool_call.id,
                            result_length = result.len(),
                            "Tool execution completed successfully"
                        );
                        ToolResult {
                            id: tool_call.id,
                            success: true,
                            result,
                            error: None,
                        }
                    }
                    Err(error) => {
                        error!(
                            event = "tool_execution_error",
                            tool_name = %tool_call.name,
                            tool_id = %tool_call.id,
                            error = %error,
                            "Tool execution failed"
                        );
                        ToolResult {
                            id: tool_call.id,
                            success: false,
                            result: String::new(),
                            error: Some(error),
                        }
                    }
                }
            }
            None => {
                error!(
                    event = "tool_not_found",
                    tool_name = %tool_call.name,
                    tool_id = %tool_call.id,
                    "Tool not found"
                );
                ToolResult {
                    id: tool_call.id,
                    success: false,
                    result: String::new(),
                    error: Some(format!("Tool '{}' not found", tool_call.name)),
                }
            }
        };

        result
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }
}