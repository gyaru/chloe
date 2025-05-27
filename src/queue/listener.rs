use redis::{Client, Commands, RedisResult};
use tracing::{info, error, warn};
use tokio::time::{sleep, Duration};
use tokio_postgres::Client as PgClient;
use std::sync::Arc;
use crate::settings::Settings;
use super::{update_prompt, settings_update};

pub struct QueueListener {
    client: Client,
    postgres_client: Arc<PgClient>,
    settings: Settings,
}

impl QueueListener {
    pub fn new(client: Client, postgres_client: Arc<PgClient>, settings: Settings) -> Self {
        Self { client, postgres_client, settings }
    }

    pub async fn start_listening(&self) {
        info!("starting chloe queue listener");
        loop {
            match self.process_queue().await {
                Ok(_) => {},
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
                    },
                    "updateSettings" => {
                        settings_update::handle_update_settings(message, &self.postgres_client, &self.settings).await;
                    },
                    _ => {
                        warn!("Unknown message type received: {}", message);
                    }
                }
            }
        }
        
        Ok(())
    }
}