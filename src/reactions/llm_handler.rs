use crate::services::{
    guild_service::GuildService,
    llm_service::{ConversationContext, LlmService, MessageContext, UserInfo},
};
use crate::utils::{ImageProcessor, MessageSanitizer};
use serenity::{async_trait, model::channel::Message, prelude::*};
use std::{collections::HashSet, sync::Arc};
use tracing::{error, info};

pub struct LLMHandler {
    pub guild_service: Arc<GuildService>,
    pub llm_service: Arc<LlmService>,
}

#[async_trait]
impl EventHandler for LLMHandler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        // Check for random reply first
        if let Some(guild_id) = msg.guild_id {
            if let Some(random_reply_setting) = self
                .guild_service
                .get_guild_setting(guild_id.get() as i64, "randomReply")
                .await
            {
                if let Some(channel_array) = random_reply_setting.as_array() {
                    for channel_value in channel_array {
                        if let Some(channel_str) = channel_value.as_str() {
                            if let Ok(target_channel_id) = channel_str.parse::<u64>() {
                                if msg.channel_id.get() == target_channel_id {
                                    // 1 in 100 chance to respond
                                    let random_number: u32 = rand::random::<u32>() % 100 + 1;

                                    if random_number == 1 {
                                        info!(
                                            event = "random_reply_triggered",
                                            user = %msg.author.name,
                                            guild_id = %guild_id,
                                            channel_id = %msg.channel_id,
                                            "Random reply triggered (1/30 chance)"
                                        );

                                        // Process this message with LLM if LLM is enabled
                                        if let Some(llm_setting) = self
                                            .guild_service
                                            .get_guild_setting(guild_id.get() as i64, "llm")
                                            .await
                                        {
                                            if llm_setting.as_bool().unwrap_or(false) {
                                                self.process_llm_message_silent(
                                                    ctx.clone(),
                                                    msg.clone(),
                                                )
                                                .await;
                                                return;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let should_respond = msg.mentions_me(&ctx.http).await.unwrap_or(false)
            || msg.content.to_lowercase().contains("chloe")
            || (msg
                .referenced_message
                .as_ref()
                .map(|ref_msg| ref_msg.author.id == ctx.cache.current_user().id)
                .unwrap_or(false));

        if should_respond {
            self.process_llm_message(ctx, msg).await;
        }
    }
}

impl LLMHandler {
    pub fn new(guild_service: Arc<GuildService>, llm_service: Arc<LlmService>) -> Self {
        Self {
            guild_service,
            llm_service,
        }
    }

    async fn process_llm_message(&self, ctx: Context, msg: Message) {
        self.process_llm_message_with_error_handling(ctx, msg, true, false)
            .await;
    }

    async fn process_llm_message_silent(&self, ctx: Context, msg: Message) {
        self.process_llm_message_with_error_handling(ctx, msg, false, true)
            .await;
    }

    async fn process_llm_message_with_error_handling(
        &self,
        ctx: Context,
        msg: Message,
        send_error_response: bool,
        is_random_reply: bool,
    ) {
        if let Some(guild_id) = msg.guild_id {
            let guild_service = Arc::clone(&self.guild_service);
            let llm_service = Arc::clone(&self.llm_service);
            let http = Arc::clone(&ctx.http);
            let msg_clone = msg;

            tokio::spawn(async move {
                if let Some(llm_setting) = guild_service
                    .get_guild_setting(guild_id.get() as i64, "llm")
                    .await
                {
                    if llm_setting.as_bool().unwrap_or(false) {
                        info!(
                            event = "llm_response_triggered",
                            user = %msg_clone.author.name,
                            guild_id = %guild_id,
                            channel_id = %msg_clone.channel_id,
                            "LLM enabled - responding to message from user"
                        );

                        let _typing = msg_clone.channel_id.start_typing(&http);
                        info!(
                            event = "typing_indicator_started",
                            user = %msg_clone.author.name,
                            channel_id = %msg_clone.channel_id,
                            "Started typing indicator"
                        );

                        // create a helper to handle image processing in the async closure
                        let image_processor = ImageProcessor::new();

                        let reply_chain_messages = image_processor
                            .get_reply_chain_context(&http, &msg_clone)
                            .await;

                        info!(
                            event = "reply_chain_context_gathered",
                            user = %msg_clone.author.name,
                            channel_id = %msg_clone.channel_id,
                            message_count = reply_chain_messages.len(),
                            messages = ?reply_chain_messages.iter().map(|m| format!("{}: {}", m.user_display_name, m.content)).collect::<Vec<_>>(),
                            "Gathered reply chain context"
                        );
                        let user_display_name = msg_clone
                            .author_nick(&http)
                            .await
                            .unwrap_or_else(|| msg_clone.author.display_name().to_string());

                        // process images from the current message
                        let current_images =
                            image_processor.process_message_images(&msg_clone).await;

                        let bot_user_id = ctx.cache.current_user().id.get();
                        let user_info = LLMHandler::gather_user_info(
                            &reply_chain_messages,
                            &msg_clone,
                            &user_display_name,
                            bot_user_id,
                        )
                        .await;

                        let referenced_message =
                            if let Some(ref ref_msg) = msg_clone.referenced_message {
                                let ref_user_display_name = if ref_msg.author.bot {
                                    "Chloe".to_string()
                                } else {
                                    ref_msg.author_nick(&http).await.unwrap_or_else(|| {
                                        ref_msg.author.display_name().to_string()
                                    })
                                };

                                let ref_images =
                                    image_processor.process_message_images(ref_msg).await;

                                // Sanitize referenced message content
                                let ref_sanitized_content = MessageSanitizer::sanitize_message(
                                    &ref_msg.content,
                                    &ref_user_display_name
                                );

                                Some(MessageContext {
                                    user_display_name: ref_user_display_name,
                                    user_id: ref_msg.author.id.get(),
                                    content: ref_sanitized_content,
                                    is_bot: ref_msg.author.bot,
                                    channel_id: ref_msg.channel_id.get(),
                                    images: ref_images,
                                })
                            } else {
                                None
                            };

                        // Sanitize the current message to prevent impersonation
                        let sanitized_message = MessageSanitizer::sanitize_message(
                            &msg_clone.content,
                            &user_display_name
                        );

                        let context = ConversationContext {
                            current_user: user_display_name,
                            current_message: sanitized_message,
                            current_images,
                            recent_messages: reply_chain_messages,
                            user_info,
                            referenced_message,
                            is_random_reply,
                        };

                        // create a sender for immediate responses (two-part tool calls)
                        let http_clone = Arc::clone(&http);
                        let msg_clone_for_sender = msg_clone.clone();
                        let _sender = move |initial_text: String| {
                            let http = Arc::clone(&http_clone);
                            let msg = msg_clone_for_sender.clone();
                            async move {
                                if let Err(why) = msg.reply(&http, initial_text).await {
                                    error!(
                                        event = "initial_response_send_failed",
                                        user = %msg.author.name,
                                        error = ?why,
                                        "Error sending initial LLM response"
                                    );
                                }
                            }
                        };

                        // create a typing starter for tool execution
                        let msg_clone_for_typing = msg_clone.clone();
                        let http_clone_for_typing = Arc::clone(&http);
                        let typing_starter = move || {
                            let msg = msg_clone_for_typing.clone();
                            let http = http_clone_for_typing.clone();
                            async move {
                                let _typing = msg.channel_id.start_typing(&http);
                                info!(
                                    event = "tool_execution_typing_started",
                                    user = %msg.author.name,
                                    channel_id = %msg.channel_id,
                                    "Started typing indicator for tool execution"
                                );
                            }
                        };

                        // Create Discord context for tool execution
                        let discord_context = crate::tools::DiscordContext {
                            http: Arc::clone(&http),
                            channel_id: msg_clone.channel_id,
                            message_id: msg_clone.id,
                            guild_id: msg_clone.guild_id,
                        };

                        match llm_service
                            .prompt_with_context_and_sender_with_discord(
                                context,
                                None::<fn(String) -> std::future::Ready<()>>,
                                Some(typing_starter),
                                Some(&discord_context),
                            )
                            .await
                        {
                            Ok(llm_response) => {
                                info!(
                                    event = "llm_response_received",
                                    user = %msg_clone.author.name,
                                    response_length = llm_response.raw_text.len(),
                                    "Received LLM response, Discord tools should have been executed automatically"
                                );

                                // With direct tool execution, all Discord actions should already be complete
                                // No additional processing needed - tools handled everything directly
                            }
                            Err(err) => {
                                error!(
                                    event = "llm_processing_failed",
                                    user = %msg_clone.author.name,
                                    error = ?err,
                                    send_error_response = send_error_response,
                                    "Error getting LLM response"
                                );
                                if send_error_response {
                                    if let Err(why) = msg_clone.reply(&http, "Sorry, I'm having trouble processing your message right now.").await {
                                        error!(
                                            event = "fallback_response_send_failed",
                                            user = %msg_clone.author.name,
                                            error = ?why,
                                            "Error sending fallback response"
                                        );
                                    }
                                } else {
                                    info!(
                                        event = "llm_processing_failed_silent",
                                        user = %msg_clone.author.name,
                                        "LLM processing failed for random reply, staying silent"
                                    );
                                }
                            }
                        }
                    } else {
                        info!(
                            event = "llm_disabled_for_guild",
                            guild_id = %guild_id,
                            user = %msg_clone.author.name,
                            "LLM disabled for guild, ignoring message"
                        );
                    }
                } else {
                    info!(
                        event = "llm_setting_not_found",
                        guild_id = %guild_id,
                        user = %msg_clone.author.name,
                        "No LLM setting found for guild, ignoring message"
                    );
                }
            });
        }
    }

    async fn gather_user_info(
        recent_messages: &[MessageContext],
        current_msg: &Message,
        current_user_display: &str,
        bot_user_id: u64,
    ) -> Vec<UserInfo> {
        let mut user_info = Vec::new();
        let mut seen_users = HashSet::new();

        if seen_users.insert(current_msg.author.id.get()) {
            user_info.push(UserInfo {
                display_name: current_user_display.to_string(),
                user_id: current_msg.author.id.get(),
                is_bot: current_msg.author.bot,
            });
        }

        for msg in recent_messages {
            if seen_users.insert(msg.user_id) {
                user_info.push(UserInfo {
                    display_name: msg.user_display_name.clone(),
                    user_id: msg.user_id,
                    is_bot: msg.is_bot,
                });
            }
        }

        if seen_users.insert(bot_user_id) {
            user_info.push(UserInfo {
                display_name: "Chloe".to_string(),
                user_id: bot_user_id,
                is_bot: true,
            });
        }

        user_info
    }
}
