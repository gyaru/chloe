use serenity::{async_trait, model::channel::Message, prelude::*};

pub struct MentionHandler;

#[async_trait]
impl EventHandler for MentionHandler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        if msg.mentions_me(&ctx.http).await.unwrap_or(false) {
            if let Err(why) = msg.channel_id.say(&ctx.http, "Hello! You mentioned me! 👋").await {
                println!("Error sending message: {:?}", why);
            }
        }
    }
}