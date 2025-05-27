use serenity::{async_trait, model::channel::Message, prelude::*};

pub struct ReplyHandler;

#[async_trait]
impl EventHandler for ReplyHandler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        if let Some(referenced_message) = &msg.referenced_message {
            if referenced_message.author.id == ctx.cache.current_user().id {
                if let Err(why) = msg.channel_id.say(&ctx.http, "Thanks for replying to me! 💬").await {
                    println!("Error sending message: {:?}", why);
                }
            }
        }
    }
}