use crate::services::guild_service::GuildService;
use serenity::{async_trait, model::channel::Message, prelude::*};
use std::sync::Arc;
use tracing::{error, info};

pub struct LLMHandler {
    pub guild_service: Arc<GuildService>,
}

#[async_trait]
impl EventHandler for LLMHandler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        if let Some(referenced_message) = &msg.referenced_message {
            if referenced_message.author.id == ctx.cache.current_user().id {
                info!("Responding to reply from user: {}", msg.author.name);
                if let Err(why) = msg.reply(&ctx.http, "Thanks for replying to me! 💬").await {
                    error!("Error sending reply response: {:?}", why);
                }
                return;
            }
        }

        if msg.mentions_me(&ctx.http).await.unwrap_or(false) {
            info!("Bot mentioned by user: {}", msg.author.name);
            if let Err(why) = msg.reply(&ctx.http, "Hello! You mentioned me! 👋").await {
                error!("Error sending mention response: {:?}", why);
            }
            return;
        }

        if msg.content.to_lowercase().contains("chloe") {
            info!("Bot mentioned by user: {}", msg.author.name);

            if let Some(guild_id) = msg.guild_id {
                let guild_service = Arc::clone(&self.guild_service);
                let http = Arc::clone(&ctx.http);
                let channel_id = msg.channel_id;
                let author_id = msg.author.id;

                let msg_clone = msg.clone();
                tokio::spawn(async move {
                    if let Some(response) = guild_service
                        .get_ping_response(guild_id.get() as i64, author_id.get() as i64)
                        .await
                    {
                        if let Err(why) = msg_clone.reply(&http, response).await {
                            error!("Error sending ping response: {:?}", why);
                        }
                    }
                });
            }
        }
    }
}
