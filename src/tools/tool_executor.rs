use super::{DiscordContext, Tool, ToolCall, ToolResult};
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

    pub async fn execute_tool(
        &self,
        tool_call: ToolCall,
        discord_context: Option<&DiscordContext>,
    ) -> ToolResult {
        self.execute_tool_with_smart_context(tool_call, discord_context)
            .await
    }

    pub async fn execute_tool_with_smart_context(
        &self,
        tool_call: ToolCall,
        discord_context: Option<&DiscordContext>,
    ) -> ToolResult {
        info!(
            event = "tool_execution_start",
            tool_name = %tool_call.name,
            tool_id = %tool_call.id,
            parameters = ?tool_call.parameters,
            "Starting tool execution"
        );

        let result = match self.tools.get(&tool_call.name) {
            Some(tool) => {
                // Check if this tool needs Discord context
                let context_to_pass = if tool.needs_discord_context() {
                    if discord_context.is_none() {
                        return ToolResult {
                            id: tool_call.id,
                            success: false,
                            result: String::new(),
                            error: Some(format!(
                                "Tool '{}' requires Discord context but none was provided",
                                tool_call.name
                            )),
                        };
                    }
                    discord_context
                } else {
                    None // Don't pass Discord context for tools that don't need it
                };

                match tool.execute(tool_call.parameters, context_to_pass).await {
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

    pub fn tool_needs_result_feedback(&self, name: &str) -> bool {
        self.tools
            .get(name)
            .map(|tool| tool.needs_result_feedback())
            .unwrap_or(true) // Default to true if tool not found
    }

    pub async fn execute_tool_by_name(
        &self,
        tool_name: &str,
        args: serde_json::Value,
        discord_context: Option<&DiscordContext>,
    ) -> Result<String, String> {
        let tool = match self.tools.get(tool_name) {
            Some(tool) => tool,
            None => return Err(format!("Tool '{}' not found", tool_name)),
        };

        info!(
            event = "tool_execution_starting",
            tool_name = %tool_name,
            "Starting tool execution"
        );

        // Convert args to HashMap
        let parameters = match args.as_object() {
            Some(obj) => obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            None => return Err("Tool arguments must be an object".to_string()),
        };

        match tool.execute(parameters, discord_context).await {
            Ok(result) => {
                info!(
                    event = "tool_execution_success",
                    tool_name = %tool_name,
                    result_length = result.len(),
                    "Tool execution completed successfully"
                );
                Ok(result)
            }
            Err(error) => {
                error!(
                    event = "tool_execution_error",
                    tool_name = %tool_name,
                    error = %error,
                    "Tool execution failed"
                );
                Err(error)
            }
        }
    }
}
