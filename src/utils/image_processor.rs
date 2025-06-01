use crate::services::llm_service::{MessageContext, ImageData};
use serenity::model::channel::Message;
use std::sync::Arc;
use tracing::{error, info};

pub struct ImageProcessor {
    http_client: reqwest::Client,
}

impl ImageProcessor {
    pub fn new() -> Self {
        Self {
            http_client: reqwest::Client::new(),
        }
    }

    pub async fn download_and_encode_image(&self, url: &str) -> Result<ImageData, Box<dyn std::error::Error + Send + Sync>> {
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

    pub async fn process_message_images(&self, msg: &Message) -> Vec<ImageData> {
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

    pub async fn get_reply_chain_context(
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
        
        // follow the reply chain up to 15 messages
        while let Some(msg) = msg_to_follow {
            if reply_chain.len() >= 15 {
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
        if reply_chain.len() < 8 {
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
        _existing_chain: &[MessageContext],
    ) -> Result<Vec<MessageContext>, Box<dyn std::error::Error + Send + Sync>> {
        let mut context = Vec::new();
        
        // fetch recent messages from the channel
        let messages = current_msg.channel_id.messages(http, serenity::builder::GetMessages::new().before(current_msg.id).limit(20)).await?;
        
        for msg in messages.iter().take(12) {
            
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
            
            if context.len() >= 8 {
                break;
            }
        }
        
        Ok(context)
    }
}