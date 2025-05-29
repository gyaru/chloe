use crate::services::{
    guild_service::GuildService,
    llm_service::{ConversationContext, LlmService, MessageContext, UserInfo, ImageData},
};
use serenity::{async_trait, model::channel::Message, prelude::*};
use std::{collections::HashSet, sync::Arc};
use tracing::{error, info};
use reqwest::Client;

pub struct LLMHandler {
    pub guild_service: Arc<GuildService>,
    pub llm_service: Arc<LlmService>,
    http_client: Client,
}

struct ImageProcessor {
    http_client: Client,
}

impl ImageProcessor {
    async fn download_and_encode_image(&self, url: &str) -> Result<ImageData, Box<dyn std::error::Error + Send + Sync>> {
        info!(
            event = "downloading_image",
            url = url,
            "Downloading image from Discord"
        );

        let response = self.http_client.get(url).send().await?;
        let content_type = response.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/jpeg")
            .to_string();
        
        let bytes = response.bytes().await?;
        let base64_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
        
        info!(
            event = "image_encoded",
            url = url,
            mime_type = %content_type,
            size_bytes = bytes.len(),
            "Successfully encoded image to base64"
        );

        Ok(ImageData {
            base64_data,
            mime_type: content_type,
        })
    }

    async fn process_message_images(&self, msg: &Message) -> Vec<ImageData> {
        let mut images = Vec::new();
        
        for attachment in &msg.attachments {
            if attachment.content_type.as_ref()
                .map(|ct| ct.starts_with("image/"))
                .unwrap_or(false) 
            {
                match self.download_and_encode_image(&attachment.url).await {
                    Ok(image_data) => {
                        info!(
                            event = "image_processed",
                            attachment_id = attachment.id.get(),
                            filename = %attachment.filename,
                            "Successfully processed image attachment"
                        );
                        images.push(image_data);
                    }
                    Err(e) => {
                        error!(
                            event = "image_processing_failed",
                            attachment_id = attachment.id.get(),
                            filename = %attachment.filename,
                            error = ?e,
                            "Failed to process image attachment"
                        );
                    }
                }
            }
        }
        
        images
    }

    async fn get_reply_chain_context(
        &self,
        http: &Arc<serenity::http::Http>,
        current_msg: &Message,
    ) -> Vec<MessageContext> {
        let mut reply_chain = Vec::new();
        let mut msg_to_follow = current_msg.referenced_message.as_ref().map(|m| m.as_ref());
        
        info!(
            event = "starting_reply_chain_trace",
            current_msg_id = current_msg.id.get(),
            has_referenced_msg = msg_to_follow.is_some(),
            "Starting to trace reply chain"
        );
        
        // follow the reply chain up to 5 messages
        while let Some(msg) = msg_to_follow {
            if reply_chain.len() >= 5 {
                break;
            }
            
            info!(
                event = "processing_chain_message",
                msg_id = msg.id.get(),
                content = %msg.content,
                author = %msg.author.name,
                has_next_ref = msg.referenced_message.is_some(),
                next_ref_id = msg.referenced_message.as_ref().map(|m| m.id.get()),
                "Processing message in reply chain"
            );
            
            if msg.content.is_empty() {
                info!(
                    event = "skipping_empty_message",
                    msg_id = msg.id.get(),
                    "Skipping empty message in chain"
                );
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
            
            let images = self.process_message_images(msg).await;
            
            reply_chain.push(MessageContext {
                user_display_name,
                user_id: msg.author.id.get(),
                content: msg.content.clone(),
                is_bot: msg.author.bot,
                channel_id: msg.channel_id.get(),
                images,
            });
            
            // Follow the chain if this message is also a reply
            if let Some(ref_msg) = &msg.referenced_message {
                info!(
                    event = "found_next_reference",
                    current_msg_id = msg.id.get(),
                    next_ref_id = ref_msg.id.get(),
                    "Found next message in chain"
                );
                msg_to_follow = Some(ref_msg.as_ref());
            } else {
                info!(
                    event = "chain_end_reached",
                    current_msg_id = msg.id.get(),
                    "No more references found, ending chain"
                );
                msg_to_follow = None;
            }
        }
        
        // if we have a short chain, try to supplement with recent channel history
        if reply_chain.len() < 3 {
            info!(
                event = "supplementing_with_channel_history",
                chain_length = reply_chain.len(),
                "Reply chain is short, fetching recent channel history"
            );
            
            match self.get_recent_channel_context(http, current_msg, &reply_chain).await {
                Ok(mut additional_context) => {
                    additional_context.extend(reply_chain);
                    reply_chain = additional_context;
                    info!(
                        event = "supplemented_context",
                        new_chain_length = reply_chain.len(),
                        "Successfully supplemented with channel history"
                    );
                }
                Err(e) => {
                    info!(
                        event = "failed_to_supplement",
                        error = ?e,
                        "Failed to fetch channel history"
                    );
                }
            }
        }
        
        info!(
            event = "reply_chain_complete",
            chain_length = reply_chain.len(),
            "Completed reply chain tracing"
        );
        
        // reverse to get chronological order (oldest first) and return the chain
        if !reply_chain.is_empty() {
            reply_chain.reverse();
        }
        reply_chain
    }

    async fn get_recent_channel_context(
        &self,
        http: &Arc<serenity::http::Http>,
        current_msg: &Message,
        existing_chain: &[MessageContext],
    ) -> Result<Vec<MessageContext>, Box<dyn std::error::Error + Send + Sync>> {
        let mut context = Vec::new();
        let existing_ids: std::collections::HashSet<u64> = existing_chain.iter().map(|m| m.user_id).collect();
        
        // fetch recent messages from the channel
        let messages = current_msg.channel_id.messages(http, serenity::builder::GetMessages::new().before(current_msg.id).limit(10)).await?;
        
        for msg in messages.iter().take(5) {
            // skip if we already have this message in the chain
            if existing_ids.contains(&msg.author.id.get()) {
                continue;
            }
            
            if msg.content.is_empty() || msg.author.bot && msg.author.id != http.get_current_user().await?.id {
                continue;
            }
            
            let user_display_name = if msg.author.bot {
                "Chloe".to_string()
            } else {
                msg.author_nick(http).await.unwrap_or_else(|| {
                    msg.author.display_name().to_string()
                })
            };
            
            let images = self.process_message_images(msg).await;
            
            context.push(MessageContext {
                user_display_name,
                user_id: msg.author.id.get(),
                content: msg.content.clone(),
                is_bot: msg.author.bot,
                channel_id: msg.channel_id.get(),
                images,
            });
            
            if context.len() >= 3 {
                break;
            }
        }
        
        Ok(context)
    }
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
                let http_client = self.http_client.clone();
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

                            // create a helper to handle image processing in the async closure
                            let image_processor = ImageProcessor { http_client: http_client.clone() };
                            
                            let reply_chain_messages =
                                image_processor.get_reply_chain_context(&http, &msg_clone)
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

                            // process images from the current message
                            let current_images = image_processor.process_message_images(&msg_clone).await;

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
                                
                                let ref_images = image_processor.process_message_images(ref_msg).await;
                                
                                Some(MessageContext {
                                    user_display_name: ref_user_display_name,
                                    user_id: ref_msg.author.id.get(),
                                    content: ref_msg.content.clone(),
                                    is_bot: ref_msg.author.bot,
                                    channel_id: ref_msg.channel_id.get(),
                                    images: ref_images,
                                })
                            } else {
                                None
                            };

                            let context = ConversationContext {
                                current_user: user_display_name,
                                current_message: msg_clone.content.clone(),
                                current_images,
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
    pub fn new(guild_service: Arc<GuildService>, llm_service: Arc<LlmService>) -> Self {
        Self {
            guild_service,
            llm_service,
            http_client: Client::new(),
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
