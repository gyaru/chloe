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
        info!("Loading global settings from database...");
        self.load_global_settings(db_pool).await
    }

    pub async fn get_global_settings(&self) -> GlobalSettings {
        let data = self.global_data.read().await;
        data.clone()
    }

    pub async fn reload_from_database(&self, db_pool: &PgPool) -> Result<(), sqlx::Error> {
        info!("Reloading settings from database...");
        self.load_from_database(db_pool).await
    }

    pub async fn load_global_settings(&self, db_pool: &PgPool) -> Result<(), sqlx::Error> {
        if let Ok(row) = sqlx::query("SELECT prompt FROM chloe_settings WHERE id = 1")
            .fetch_one(db_pool)
            .await
        {
            let prompt: String = row.get("prompt");
            let mut data = self.global_data.write().await;
            data.prompt = prompt.clone();
            info!("Global settings loaded: prompt = '{}'", prompt);
        } else {
            info!("No global settings found, using defaults");
        }
        Ok(())
    }

    pub async fn reload_global_settings(&self, db_pool: &PgPool) -> Result<(), sqlx::Error> {
        info!("Reloading global settings from database...");
        self.load_global_settings(db_pool).await
    }
}
