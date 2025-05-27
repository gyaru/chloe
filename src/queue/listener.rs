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
        info!(
            event = "queue_listener_started",
            "Starting chloe queue listener"
        );
        loop {
            match self.process_queue().await {
                Ok(_) => {}
                Err(e) => {
                    error!(
                        event = "queue_processing_error",
                        error = ?e,
                        "Error processing queue"
                    );
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

                info!(
                    event = "queue_message_received",
                    message_type = %message,
                    queue = "chloe-queue",
                    "Fetched message from queue"
                );

                match message.as_str() {
                    "updatePrompt" => {
                        update_prompt::handle_update_prompt(message).await;
                    }
                    "updateSettings" => {
                        let db_pool = self.db_pool.clone();
                        let settings = self.settings.clone();
                        let guild_service = Arc::clone(&self.guild_service);
                        let message = message.to_string();
                        
                        // Spawn settings update in background to avoid blocking queue listener
                        tokio::spawn(async move {
                            settings_update::handle_update_settings(
                                &message,
                                &db_pool,
                                &settings,
                                &guild_service,
                            )
                            .await;
                        });
                    }
                    _ => {
                        warn!(
                            event = "unknown_queue_message",
                            message_type = %message,
                            "Unknown message type received"
                        );
                    }
                }
            }
        }

        Ok(())
    }
}
