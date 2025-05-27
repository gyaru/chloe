use serenity::{async_trait, model::channel::Message, prelude::*};
use tracing::{info, error};

pub struct CombinedHandler;

#[async_trait]
impl EventHandler for CombinedHandler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        if let Some(referenced_message) = &msg.referenced_message {
            if referenced_message.author.id == ctx.cache.current_user().id {
                info!("Responding to reply from user: {}", msg.author.name);
                if let Err(why) = msg.channel_id.say(&ctx.http, "Thanks for replying to me! 💬").await {
                    error!("Error sending reply response: {:?}", why);
                }
                return; // exit early to prevent other handlers from triggering
            }
        }

        // check if bot is mentioned
        if msg.mentions_me(&ctx.http).await.unwrap_or(false) {
            info!("Bot mentioned by user: {}", msg.author.name);
            if let Err(why) = msg.channel_id.say(&ctx.http, "Hello! You mentioned me! 👋").await {
                error!("Error sending mention response: {:?}", why);
            }
            return;
        }

        if msg.content.to_lowercase().contains("ping") {
            info!("Ping detected from user: {}", msg.author.name);
            if let Err(why) = msg.channel_id.say(&ctx.http, "Pong!").await {
                error!("Error sending ping response: {:?}", why);
            }
        }
    }
}