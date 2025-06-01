// Individual tool modules
pub mod time;
pub mod web_search;
pub mod playwright;
pub mod image_generation;
pub mod discord_message;
pub mod discord_reaction;
pub mod calculator;

// Core tool infrastructure
pub mod tool_executor;

// Re-export all tools for easy access
pub use time::GetTimeTool;
pub use web_search::WebSearchTool;
pub use playwright::PlaywrightWebContentTool;
pub use image_generation::ImageGenerationTool;
pub use discord_message::DiscordSendMessageTool;
pub use discord_reaction::DiscordAddReactionTool;
pub use calculator::CalculatorTool;

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

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
    async fn execute(&self, parameters: HashMap<String, Value>, discord_context: Option<&DiscordContext>) -> Result<String, String>;
}