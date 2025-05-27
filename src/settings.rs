use sqlx::{PgPool, Row};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

#[derive(Clone)]
pub struct Settings {
    global_data: Arc<RwLock<GlobalSettings>>,
}

#[derive(Clone, Debug)]
pub struct GlobalSettings {
    pub prompt: String,
}

impl Settings {
    pub fn new() -> Self {
        Self {
            global_data: Arc::new(RwLock::new(GlobalSettings {
                prompt: "You're Chloe, a discord bot.".to_string(),
            })),
        }
    }

    pub async fn load_from_database(&self, db_pool: &PgPool) -> Result<(), sqlx::Error> {
        info!(
            event = "settings_load_started",
            "Loading global settings from database"
        );
        self.load_global_settings(db_pool).await
    }

    pub async fn get_global_settings(&self) -> GlobalSettings {
        let data = self.global_data.read().await;
        data.clone()
    }

    pub async fn reload_from_database(&self, db_pool: &PgPool) -> Result<(), sqlx::Error> {
        info!(
            event = "settings_reload_started",
            "Reloading settings from database"
        );
        self.load_from_database(db_pool).await
    }

    pub async fn load_global_settings(&self, db_pool: &PgPool) -> Result<(), sqlx::Error> {
        info!(
            event = "global_settings_loading",
            "Starting to load global settings from database"
        );
        
        if let Ok(row) = sqlx::query("SELECT prompt FROM chloe_settings WHERE id = 1")
            .fetch_one(db_pool)
            .await
        {
            let prompt: String = row.get("prompt");
            
            info!(
                event = "global_settings_db_loaded",
                prompt_length = prompt.len(),
                "Loaded prompt from database, acquiring write lock"
            );
            
            // try to get write lock with timeout to detect deadlocks
            match tokio::time::timeout(
                tokio::time::Duration::from_secs(5),
                self.global_data.write()
            ).await {
                Ok(mut data) => {
                    data.prompt = prompt.clone();
                    info!(
                        event = "global_settings_loaded",
                        prompt_length = prompt.len(),
                        "Global settings loaded from database"
                    );
                }
                Err(_) => {
                    tracing::error!(
                        event = "global_settings_write_lock_timeout",
                        "Failed to acquire write lock for global settings - possible deadlock"
                    );
                    return Err(sqlx::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "Write lock timeout"
                    )));
                }
            }
        } else {
            info!(
                event = "global_settings_default",
                "No global settings found, using defaults"
            );
        }
        Ok(())
    }

    pub async fn reload_global_settings(&self, db_pool: &PgPool) -> Result<(), sqlx::Error> {
        info!(
            event = "global_settings_reload_started",
            "Reloading global settings from database"
        );
        self.load_global_settings(db_pool).await
    }
}
