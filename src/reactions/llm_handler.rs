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

                            let reply_chain_messages =
                                Self::get_reply_chain_context(&http, &msg_clone)
                                    .await;

                            info!(
                                event = "reply_chain_context_gathered",
                                user = %msg_clone.author.name,
                                channel_id = %msg_clone.channel_id,
                                message_count = reply_chain_messages.len(),
                                messages = ?reply_chain_messages.iter().map(|m| format!("{}: {}", m.user_display_name, m.content)).collect::<Vec<_>>(),
                                "Gathered reply chain context"
                            );
                            let user_display_name =
                                msg_clone.author_nick(&http).await.unwrap_or_else(|| {
                                    msg_clone
                                        .author
                                        .display_name().to_string()
                                });

                            let bot_user_id = ctx.cache.current_user().id.get();
                            let user_info = Self::gather_user_info(
                                &reply_chain_messages,
                                &msg_clone,
                                &user_display_name,
                                bot_user_id,
                            )
                            .await;

                            let referenced_message = if let Some(ref ref_msg) = msg_clone.referenced_message {
                                let ref_user_display_name = if ref_msg.author.bot {
                                    "Chloe".to_string()
                                } else {
                                    ref_msg.author_nick(&http).await.unwrap_or_else(|| {
                                        ref_msg.author.display_name().to_string()
                                    })
                                };
                                
                                Some(MessageContext {
                                    user_display_name: ref_user_display_name,
                                    user_id: ref_msg.author.id.get(),
                                    content: ref_msg.content.clone(),
                                    is_bot: ref_msg.author.bot,
                                    channel_id: ref_msg.channel_id.get(),
                                })
                            } else {
                                None
                            };

                            let context = ConversationContext {
                                current_user: user_display_name,
                                current_message: msg_clone.content.clone(),
                                recent_messages: reply_chain_messages,
                                user_info,
                                referenced_message,
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
    async fn get_reply_chain_context(
        http: &Arc<serenity::http::Http>,
        current_msg: &Message,
    ) -> Vec<MessageContext> {
        let mut reply_chain = Vec::new();
        let mut msg_to_follow = current_msg.referenced_message.as_ref().map(|m| m.as_ref());
        
        // follow the reply chain up to 5 messages
        while let Some(msg) = msg_to_follow {
            if reply_chain.len() >= 5 {
                break;
            }
            
            if msg.content.is_empty() {
                msg_to_follow = msg.referenced_message.as_ref().map(|m| m.as_ref());
                continue;
            }
            
            let user_display_name = if msg.author.bot {
                "Chloe".to_string()
            } else {
                msg.author_nick(http).await.unwrap_or_else(|| {
                    msg.author.display_name().to_string()
                })
            };
            
            reply_chain.push(MessageContext {
                user_display_name,
                user_id: msg.author.id.get(),
                content: msg.content.clone(),
                is_bot: msg.author.bot,
                channel_id: msg.channel_id.get(),
            });
            
            // Follow the chain if this message is also a reply
            msg_to_follow = msg.referenced_message.as_ref().map(|m| m.as_ref());
        }
        
        // only return the chain if it has at least 2 messages
        if reply_chain.len() >= 2 {
            // reverse to get chronological order (oldest first)
            reply_chain.reverse();
            reply_chain
        } else {
            Vec::new()
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
