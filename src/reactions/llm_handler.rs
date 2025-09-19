use crate::services::{guild_service::GuildService, llm_service::LlmService};
use crate::utils::MessageSanitizer;
use serenity::{async_trait, model::channel::Message, prelude::*};
use std::sync::Arc;
use tracing::{error, info};

pub struct LlmHandler {
    pub guild_service: Arc<GuildService>,
    pub llm_service: Arc<LlmService>,
}

impl LlmHandler {
    pub fn new(guild_service: Arc<GuildService>, llm_service: Arc<LlmService>) -> Self {
        Self {
            guild_service,
            llm_service,
        }
    }
}

#[async_trait]
impl EventHandler for LlmHandler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        // Check if bot was mentioned or message is a reply to the bot
        let bot_mentioned = msg.mentions_user_id(ctx.cache.current_user().id);
        let reply_to_bot = if let Some(referenced_message) = &msg.referenced_message {
            referenced_message.author.id == ctx.cache.current_user().id
        } else {
            false
        };

        if !bot_mentioned && !reply_to_bot {
            return;
        }

        // Check if LLM is enabled for this guild
        if let Some(guild_id) = msg.guild_id {
            if let Some(llm_setting) = self
                .guild_service
                .get_guild_setting(guild_id.get() as i64, "llm")
                .await
            {
                if !llm_setting.as_bool().unwrap_or(false) {
                    return;
                }
            } else {
                return;
            }
        }

        info!(
            event = "llm_message_received",
            user = %msg.author.name,
            guild_id = ?msg.guild_id,
            channel_id = %msg.channel_id,
            mentioned = bot_mentioned,
            reply = reply_to_bot,
            "Processing message for LLM response"
        );

        let ctx_clone = ctx.clone();
        let msg_clone = msg.clone();
        let llm_service_clone = Arc::clone(&self.llm_service);

        tokio::spawn(async move {
            Self::handle_llm_response(ctx_clone, msg_clone, llm_service_clone).await;
        });
    }
}

impl LlmHandler {
    async fn handle_llm_response(ctx: Context, msg: Message, llm_service: Arc<LlmService>) {
        // Start typing
        let _typing = msg.channel_id.start_typing(&ctx.http);

        // Get system prompt from settings
        let global_settings = llm_service.settings.get_global_settings().await;
        let system_prompt = &global_settings.prompt;

        // Sanitize user message
        let user_message = MessageSanitizer::sanitize_message(
            &msg.content,
            &msg.author.display_name().to_string(),
        );

        // Generate response
        match llm_service
            .generate_response(system_prompt, &user_message)
            .await
        {
            Ok(response) => {
                info!(
                    event = "llm_response_success",
                    provider = llm_service.get_provider_name(),
                    "Successfully generated LLM response"
                );

                // Send response
                let sanitized_response = MessageSanitizer::sanitize_for_discord(&response.text);
                if let Err(e) = msg.channel_id.say(&ctx.http, sanitized_response).await {
                    error!(
                        event = "message_send_failed",
                        error = ?e,
                        channel_id = %msg.channel_id,
                        "Failed to send message"
                    );
                }
            }
            Err(e) => {
                error!(
                    event = "llm_response_failed",
                    provider = llm_service.get_provider_name(),
                    error = ?e,
                    "Failed to generate LLM response"
                );

                // Send fallback error message
                let fallback_message = "Sorry, I encountered an error while processing your message. Please try again later.";
                if let Err(send_err) = msg.channel_id.say(&ctx.http, fallback_message).await {
                    error!(
                        event = "fallback_message_failed",
                        error = ?send_err,
                        "Failed to send fallback error message"
                    );
                }
            }
        }
    }
}
