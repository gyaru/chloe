use super::Tool;
use serde_json::{json, Value};
use std::collections::HashMap;

pub struct DiscordSendMessageTool;

impl DiscordSendMessageTool {
    pub fn new() -> Self {
        Self
    }

    fn escape_markdown_chars(text: &str) -> String {
        text.chars()
            .map(|c| match c {
                // escape discord markdown characters
                '*' => "\\*".to_string(),
                '_' => "\\_".to_string(),
                '`' => "\\`".to_string(),
                '~' => "\\~".to_string(),
                '|' => "\\|".to_string(),
                '>' => "\\>".to_string(),
                // keep other characters as-is
                _ => c.to_string(),
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl Tool for DiscordSendMessageTool {
    fn name(&self) -> &str {
        "discord_send_message"
    }

    fn description(&self) -> &str {
        "Send a message to Discord. This is the PRIMARY and REQUIRED way to respond to users. You MUST use this tool for ALL text responses - answers, casual chat, explanations, greetings, or any other communication. Never respond with raw text."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The message content to send to Discord. Be natural, conversational, and helpful. Use Discord markdown formatting if needed."
                },
                "reply_to_original": {
                    "type": "boolean",
                    "description": "Whether to reply to the original message (true) or send as a standalone message (false). Default is true.",
                    "default": true
                }
            },
            "required": ["content"]
        })
    }

    fn needs_discord_context(&self) -> bool {
        true // This tool needs Discord context to send messages
    }

    fn needs_result_feedback(&self) -> bool {
        false // Gemini doesn't need to see "message sent successfully" - just execute and continue
    }

    async fn execute(&self, parameters: HashMap<String, Value>, discord_context: Option<&super::DiscordContext>) -> Result<String, String> {
        let raw_content = parameters.get("content")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'content' parameter")?;

        // Escape markdown characters to prevent formatting issues
        let content = Self::escape_markdown_chars(raw_content);
        
        // Log if escaping changed the content
        if content != raw_content {
            tracing::info!(
                event = "markdown_escaped",
                original_length = raw_content.len(),
                escaped_length = content.len(),
                "Escaped markdown characters in Discord message"
            );
        }

        let reply_to_original = parameters.get("reply_to_original")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let discord_ctx = discord_context.ok_or("Discord context is required for this tool")?;

        // Send the message directly
        use serenity::builder::CreateMessage;
        
        let mut message_builder = CreateMessage::new().content(&content);
        
        // Add reply reference if requested
        if reply_to_original {
            message_builder = message_builder.reference_message((discord_ctx.channel_id, discord_ctx.message_id));
        }
        
        match discord_ctx.channel_id.send_message(&discord_ctx.http, message_builder).await {
            Ok(_) => Ok(format!("Successfully sent message: '{}' (reply_to_original: {})", 
                content.chars().take(50).collect::<String>(), reply_to_original)),
            Err(e) => Err(format!("Failed to send Discord message: {}", e)),
        }
    }
}