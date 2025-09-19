use super::Tool;
use crate::utils::regex_patterns::{
    EMOTICON_REGEX, MENTION_REGEX as DISCORD_MENTION_REGEX, URL_REGEX,
};
use serde_json::{Value, json};
use std::collections::HashMap;

pub struct DiscordSendMessageTool;

impl DiscordSendMessageTool {
    pub fn new() -> Self {
        Self
    }

    fn escape_markdown_chars(text: &str) -> String {
        // First, convert literal \n to actual newlines
        let text_with_newlines = text.replace("\\n", "\n");

        // Then check if there are any escaped mentions and fix them
        let unescaped_mentions = text_with_newlines
            .replace(r"\<@", "<@")
            .replace(r"\<#", "<#")
            .replace(r"\<&", "<&");

        // Collect all patterns to preserve
        let mut preservable_items = Vec::new();

        // Find all Discord mentions
        for m in DISCORD_MENTION_REGEX.find_iter(&unescaped_mentions) {
            preservable_items.push((m.start(), m.end(), m.as_str().to_string()));
        }

        // Find all URLs
        for m in URL_REGEX.find_iter(&unescaped_mentions) {
            preservable_items.push((m.start(), m.end(), m.as_str().to_string()));
        }

        // Find all emoticons
        for m in EMOTICON_REGEX.find_iter(&unescaped_mentions) {
            preservable_items.push((m.start(), m.end(), m.as_str().to_string()));
        }

        // Sort by position in reverse order for processing
        preservable_items.sort_by_key(|&(start, _, _)| std::cmp::Reverse(start));

        // Replace preservable items with placeholders
        let mut working_text = unescaped_mentions.clone();
        let mut placeholders = Vec::new();

        // Process in the already reversed order to maintain positions
        for (i, &(start, end, ref content)) in preservable_items.iter().enumerate() {
            let placeholder = format!("§PRESERVE§{}§", i);
            working_text.replace_range(start..end, &placeholder);
            placeholders.push((placeholder.clone(), content.clone()));
        }

        // Escape markdown characters
        let escaped = working_text
            .chars()
            .map(|c| match c {
                // escape discord markdown characters
                '*' => "\\*".to_string(),
                '_' => "\\_".to_string(),
                '`' => "\\`".to_string(),
                '~' => "\\~".to_string(),
                '|' => "\\|".to_string(),
                '>' => "\\>".to_string(),
                // keep other characters as-is (including newlines)
                _ => c.to_string(),
            })
            .collect::<String>();

        // Restore all preserved items
        let mut result = escaped;
        for (placeholder, content) in placeholders.iter() {
            result = result.replace(placeholder, content);
        }

        result
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

    async fn execute(
        &self,
        parameters: HashMap<String, Value>,
        discord_context: Option<&super::DiscordContext>,
    ) -> Result<String, String> {
        let raw_content = parameters
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'content' parameter")?;

        // Check for leaked reasoning patterns and strip them
        let content_to_use = if raw_content.contains("''' storylines='''")
            || raw_content.contains("Chosen response:")
            || raw_content.contains("\\n\\nChosen response:")
        {
            // Extract just the actual message before the reasoning leak
            if let Some(idx) = raw_content.find("''' storylines='''") {
                raw_content[..idx].trim()
            } else if let Some(idx) = raw_content.find("\\n\\nChosen response:") {
                raw_content[..idx].trim()
            } else {
                // Try to extract quoted response after "Chosen response:"
                if let Some(start) = raw_content.find("Chosen response: \"") {
                    let after_quote = &raw_content[start + 18..];
                    if let Some(end) = after_quote.find("\"") {
                        &after_quote[..end]
                    } else {
                        raw_content
                    }
                } else {
                    raw_content
                }
            }
        } else {
            raw_content
        };

        // Log if we detected and cleaned leaked reasoning
        if content_to_use != raw_content {
            tracing::warn!(
                event = "gemini_reasoning_leak_detected",
                original_length = raw_content.len(),
                cleaned_length = content_to_use.len(),
                "Detected and removed leaked Gemini reasoning from message content"
            );
        }

        // Escape markdown characters to prevent formatting issues
        let content = Self::escape_markdown_chars(content_to_use);

        // Log if escaping changed the content
        if content != content_to_use {
            let mention_count = DISCORD_MENTION_REGEX.find_iter(&content).count();
            tracing::info!(
                event = "markdown_escaped",
                original_length = content_to_use.len(),
                escaped_length = content.len(),
                mentions_preserved = mention_count,
                "Escaped markdown characters in Discord message while preserving mentions"
            );
        }

        let reply_to_original = parameters
            .get("reply_to_original")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let discord_ctx = discord_context.ok_or("Discord context is required for this tool")?;

        // Send the message directly
        use serenity::builder::CreateMessage;

        let channel_id = serenity::model::id::ChannelId::new(discord_ctx.channel_id);
        let mut message_builder = CreateMessage::new().content(&content);

        // Add reply reference if requested and message_id is available
        if reply_to_original {
            if let Some(message_id) = discord_ctx.message_id {
                let original_message_id = serenity::model::id::MessageId::new(message_id);
                let message_reference = (channel_id, original_message_id);
                message_builder = message_builder.reference_message(message_reference);
            }
        }
        match channel_id
            .send_message(&discord_ctx.http, message_builder)
            .await
        {
            Ok(_) => Ok("Message sent".to_string()),
            Err(e) => Err(format!("Failed to send Discord message: {}", e)),
        }
    }
}
