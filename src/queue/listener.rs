use super::{settings_update, update_prompt};
use crate::services::guild_service::GuildService;
use crate::settings::Settings;
use redis::{Client, Commands, RedisResult};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::time::{Duration, sleep};
use tracing::{error, info, warn};

pub struct QueueListener {
    client: Client,
    db_pool: PgPool,
    settings: Settings,
    guild_service: Arc<GuildService>,
}

impl QueueListener {
    pub fn new(
        client: Client,
        db_pool: PgPool,
        settings: Settings,
        guild_service: Arc<GuildService>,
    ) -> Self {
        Self {
            client,
            db_pool,
            settings,
            guild_service,
        }
    }

    pub async fn start_listening(&self) {
        info!("starting chloe queue listener");
        loop {
            match self.process_queue().await {
                Ok(_) => {}
                Err(e) => {
                    error!("Error processing queue: {:?}", e);
                    sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn process_queue(&self) -> RedisResult<()> {
        let mut conn = self.client.get_connection()?;

        // lower this later
        let result: Option<Vec<String>> = conn.brpop("chloe-queue", 300.0)?;

        if let Some(values) = result {
            if values.len() >= 2 {
                let _queue_name = &values[0];
                let message = &values[1];

                info!("Fetched message from chloe-queue: {}", message);

                match message.as_str() {
                    "updatePrompt" => {
                        update_prompt::handle_update_prompt(message).await;
                    }
                    "updateSettings" => {
                        settings_update::handle_update_settings(
                            message,
                            &self.db_pool,
                            &self.settings,
                            &self.guild_service,
                        )
                        .await;
                    }
                    _ => {
                        warn!("Unknown message type received: {}", message);
                    }
                }
            }
        }

        Ok(())
    }
}
