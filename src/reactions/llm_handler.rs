use crate::services::{
    guild_service::GuildService,
    llm_service::{ConversationContext, LlmService, MessageContext, UserInfo},
};
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

        let should_respond = msg.mentions_me(&ctx.http).await.unwrap_or(false)
            || msg.content.to_lowercase().contains("chloe")
            || (msg
                .referenced_message
                .as_ref()
                .map(|ref_msg| ref_msg.author.id == ctx.cache.current_user().id)
                .unwrap_or(false));

        if should_respond {
            if let Some(guild_id) = msg.guild_id {
                let guild_service = Arc::clone(&self.guild_service);
                let llm_service = Arc::clone(&self.llm_service);
                let http = Arc::clone(&ctx.http);
                let msg_clone = msg.clone();

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

                            let recent_messages =
                                Self::get_recent_messages(&http, msg_clone.channel_id, &msg_clone)
                                    .await;

                            info!(
                                event = "conversation_context_gathered",
                                user = %msg_clone.author.name,
                                channel_id = %msg_clone.channel_id,
                                message_count = recent_messages.len(),
                                messages = ?recent_messages.iter().map(|m| format!("{}: {}", m.user_display_name, m.content)).collect::<Vec<_>>(),
                                "Gathered conversation context"
                            );
                            let user_display_name =
                                msg_clone.author_nick(&http).await.unwrap_or_else(|| {
                                    msg_clone
                                        .author
                                        .global_name
                                        .clone()
                                        .unwrap_or_else(|| msg_clone.author.name.clone())
                                });

                            let bot_user_id = ctx.cache.current_user().id.get();
                            let user_info = Self::gather_user_info(
                                &recent_messages,
                                &msg_clone,
                                &user_display_name,
                                bot_user_id,
                            )
                            .await;

                            let context = ConversationContext {
                                current_user: user_display_name,
                                current_message: msg_clone.content.clone(),
                                recent_messages,
                                user_info,
                            };

                            match llm_service.prompt_with_context(context).await {
                                Ok(response) => {
                                    if let Err(why) = msg_clone.reply(&http, response).await {
                                        error!(
                                            event = "llm_response_send_failed",
                                            user = %msg_clone.author.name,
                                            error = ?why,
                                            "Error sending LLM response"
                                        );
                                    }
                                }
                                Err(err) => {
                                    error!(
                                        event = "llm_processing_failed",
                                        user = %msg_clone.author.name,
                                        error = ?err,
                                        "Error getting LLM response"
                                    );
                                    if let Err(why) = msg_clone.reply(&http, "Sorry, I'm having trouble processing your message right now.").await {
                                        error!(
                                            event = "fallback_response_send_failed",
                                            user = %msg_clone.author.name,
                                            error = ?why,
                                            "Error sending fallback response"
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
    }
}

impl LLMHandler {
    async fn get_recent_messages(
        http: &Arc<serenity::http::Http>,
        channel_id: serenity::model::id::ChannelId,
        current_msg: &Message,
    ) -> Vec<MessageContext> {
        let mut recent_messages = Vec::new();

        // get 50 messages from the channel
        if let Ok(messages) = channel_id
            .messages(http, serenity::builder::GetMessages::new().limit(50))
            .await
        {
            for msg in messages.iter() {
                if msg.id == current_msg.id {
                    continue;
                }

                // skip old messages
                if msg.timestamp < current_msg.timestamp {
                    if msg.content.is_empty() {
                        continue;
                    }

                    let user_display_name = if msg.author.bot {
                        "Chloe".to_string()
                    } else {
                        msg.author_nick(http).await.unwrap_or_else(|| {
                            msg.author
                                .global_name
                                .clone()
                                .unwrap_or_else(|| msg.author.name.clone())
                        })
                    };

                    recent_messages.push(MessageContext {
                        user_display_name,
                        user_id: msg.author.id.get(),
                        content: msg.content.clone(),
                        is_bot: msg.author.bot,
                        channel_id: channel_id.get(),
                    });

                    if recent_messages.len() >= 8 {
                        break;
                    }
                }
            }
        }

        recent_messages.reverse();
        recent_messages
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
