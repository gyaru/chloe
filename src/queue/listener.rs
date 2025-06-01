use super::{settings_update, update_prompt, user_operations};
use crate::services::guild_service::GuildService;
use crate::services::user_service::UserService;
use crate::settings::Settings;
use redis::{Client, AsyncCommands, RedisResult};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::time::{Duration, sleep};
use tracing::{error, info, warn};

pub struct QueueListener {
    client: Client,
    db_pool: PgPool,
    settings: Settings,
    guild_service: Arc<GuildService>,
    user_service: Arc<UserService>,
}

impl QueueListener {
    pub fn new(
        client: Client,
        db_pool: PgPool,
        settings: Settings,
        guild_service: Arc<GuildService>,
        user_service: Arc<UserService>,
    ) -> Self {
        Self {
            client,
            db_pool,
            settings,
            guild_service,
            user_service,
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
        let mut conn = self.client.get_multiplexed_async_connection().await?;

        // lower this later
        let result: Option<Vec<String>> = conn.brpop("chloe", 300.0).await?;

        if let Some(values) = result {
            if values.len() >= 2 {
                let _queue_name = &values[0];
                let message = &values[1];

                info!(
                    event = "queue_message_received",
                    message_type = %message,
                    queue = "chloe",
                    "Fetched message from queue"
                );

                // Try to parse as JSON first
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(message) {
                    if let Some(action) = parsed.get("action").and_then(|v| v.as_str()) {
                        match action {
                            "prompt_create" | "prompt_activate" => {
                                let settings = Arc::new(self.settings.clone());
                                let db_pool = self.db_pool.clone();
                                let message = message.to_string();
                                
                                // Process directly instead of spawning to avoid timing issues
                                update_prompt::handle_update_prompt(&message, settings, &db_pool).await;
                            }
                            "reload_settings" => {
                                let db_pool = self.db_pool.clone();
                                let settings = self.settings.clone();
                                let guild_service = Arc::clone(&self.guild_service);
                                let message = message.to_string();
                                
                                // Process directly instead of spawning to avoid timing issues
                                settings_update::handle_update_settings(
                                    &message,
                                    &db_pool,
                                    &settings,
                                    &guild_service,
                                )
                                .await;
                            }
                            "auth_user" | "get_user" | "get_users" | "get_users_by_ids" | "get_user_auth" => {
                                let user_service = Arc::clone(&self.user_service);
                                let message = message.to_string();
                                
                                // Process user operations directly
                                user_operations::handle_user_operations(&message, user_service, &self.client).await;
                            }
                            _ => {
                                warn!(
                                    event = "unknown_json_action",
                                    action = %action,
                                    "Unknown action in JSON message"
                                );
                            }
                        }
                    } else {
                        warn!(
                            event = "invalid_json_message",
                            "JSON message missing 'action' field"
                        );
                    }
                } else {
                    // Fallback to string-based matching for legacy messages
                    match message.as_str() {
                        "updateSettings" => {
                            let db_pool = self.db_pool.clone();
                            let settings = self.settings.clone();
                            let guild_service = Arc::clone(&self.guild_service);
                            let message = message.to_string();
                            
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
        }

        Ok(())
    }
}
