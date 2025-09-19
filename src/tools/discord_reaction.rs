use super::Tool;
use serde_json::{Value, json};
use std::collections::HashMap;

pub struct DiscordAddReactionTool;

impl DiscordAddReactionTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for DiscordAddReactionTool {
    fn name(&self) -> &str {
        "discord_add_reaction"
    }

    fn description(&self) -> &str {
        "Add a reaction emoji to the current Discord message. You can use Unicode emojis (like 👍, ❤️, 😂) or custom guild emoji names (like :custom_emoji:). IMPORTANT: Only use custom emojis that exist in the guild - check the Available Custom Emojis section in the prompt. When in doubt, use Unicode emojis."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "emoji": {
                    "type": "string",
                    "description": "The emoji to react with. Can be Unicode emoji (👍, ❤️, 😂) or custom emoji name (:custom_emoji:)"
                }
            },
            "required": ["emoji"]
        })
    }

    fn needs_discord_context(&self) -> bool {
        true // This tool needs Discord context to add reactions
    }

    fn needs_result_feedback(&self) -> bool {
        false // Gemini doesn't need to see "reaction added" - just execute and continue
    }

    async fn execute(
        &self,
        parameters: HashMap<String, Value>,
        discord_context: Option<&super::DiscordContext>,
    ) -> Result<String, String> {
        let emoji_str = parameters
            .get("emoji")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'emoji' parameter")?;

        let discord_ctx = discord_context.ok_or("Discord context is required for this tool")?;

        // Parse emoji - either Unicode or custom guild emoji
        let reaction_type = if emoji_str.starts_with(':') && emoji_str.ends_with(':') {
            // Custom guild emoji format :name:
            let emoji_name = &emoji_str[1..emoji_str.len() - 1];

            // Get guild emojis to find the custom emoji
            if let Some(guild_id) = discord_ctx.guild_id {
                let guild_emojis = match guild_id.emojis(&discord_ctx.http).await {
                    Ok(emojis) => emojis,
                    Err(e) => return Err(format!("Failed to fetch guild emojis: {}", e)),
                };

                // Find the emoji by name
                if let Some(custom_emoji) =
                    guild_emojis.iter().find(|emoji| emoji.name == emoji_name)
                {
                    serenity::model::channel::ReactionType::Custom {
                        animated: custom_emoji.animated,
                        id: custom_emoji.id,
                        name: Some(custom_emoji.name.clone()),
                    }
                } else {
                    // Suggest common Unicode alternatives for failed custom emojis
                    let unicode_suggestion = match emoji_name.to_lowercase().as_str() {
                        "poggers" | "pog" => "😮",
                        "kekw" | "lul" | "lol" => "😂",
                        "sadge" | "sad" => "😢",
                        "pepehands" => "😭",
                        "monkas" | "nervous" => "😰",
                        "thumbsup" | "up" => "👍",
                        "thumbsdown" | "down" => "👎",
                        "heart" | "love" => "❤️",
                        "fire" => "🔥",
                        "100" | "perfect" => "💯",
                        _ => "👍", // Default fallback
                    };

                    // Return a helpful error with the Unicode suggestion
                    return Err(format!(
                        "Custom emoji '{}' not found in guild. Try using Unicode emoji '{}' instead, or check the Available Custom Emojis section for valid options.",
                        emoji_name, unicode_suggestion
                    ));
                }
            } else {
                return Err("Cannot use custom emoji outside of guild context".to_string());
            }
        } else {
            // Unicode emoji
            serenity::model::channel::ReactionType::Unicode(emoji_str.to_string())
        };

        // Add the reaction directly
        let channel_id = serenity::model::id::ChannelId::new(discord_ctx.channel_id);
        let message_id = match discord_ctx.message_id {
            Some(id) => serenity::model::id::MessageId::new(id),
            None => return Err("Message ID not available for adding reaction".to_string()),
        };

        match discord_ctx
            .http
            .create_reaction(channel_id, message_id, &reaction_type)
            .await
        {
            Ok(_) => Ok("Reaction added".to_string()),
            Err(e) => Err(format!("Failed to add Discord reaction: {}", e)),
        }
    }
}
