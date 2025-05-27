use serenity::{async_trait, model::channel::Message, prelude::*};

pub struct MessageHandler;

#[async_trait]
impl EventHandler for MessageHandler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        if msg.content.to_lowercase().contains("ping") {
            if let Err(why) = msg.channel_id.say(&ctx.http, "Pong!").await {
                println!("Error sending message: {:?}", why);
            }
        }
    }
}