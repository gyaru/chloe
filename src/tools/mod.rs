// Individual tool modules
pub mod calculator;
pub mod discord_message;
pub mod discord_reaction;
pub mod fetch;
pub mod image_generation;
pub mod time;
pub mod web_search;

// Core tool infrastructure
pub mod tool_executor;
pub mod tool_names;

// Re-export all tools for easy access
pub use discord_message::DiscordSendMessageTool;
pub use discord_reaction::DiscordAddReactionTool;
pub use fetch::FetchTool;
pub use image_generation::ImageGenerationTool;
pub use web_search::WebSearchTool;
pub use tool_names::ToolName;

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String, // Keep as String for compatibility with existing code
    pub parameters: HashMap<String, Value>,
}

impl ToolCall {
    // Helper method to parse tool name to enum
    pub fn tool_name(&self) -> Result<ToolName, String> {
        ToolName::from_str(&self.name).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub id: String,
    pub success: bool,
    pub result: String,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct DiscordContext {
    pub http: Arc<serenity::http::Http>,
    pub channel_id: serenity::model::id::ChannelId,
    pub message_id: serenity::model::id::MessageId,
    pub guild_id: Option<serenity::model::id::GuildId>,
}

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    fn needs_discord_context(&self) -> bool {
        false // Default: most tools don't need Discord context
    }
    fn needs_result_feedback(&self) -> bool {
        true // Default: most tools need their results fed back to Gemini
    }
    async fn execute(
        &self,
        parameters: HashMap<String, Value>,
        discord_context: Option<&DiscordContext>,
    ) -> Result<String, String>;
}
