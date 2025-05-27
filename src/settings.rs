use std::sync::Arc;
use tokio::sync::RwLock;
use sqlx::{PgPool, Row};
use serde_json::Value;
use tracing::info;

#[derive(Clone)]
pub struct Settings {
    data: Arc<RwLock<Option<Value>>>,
}

impl Settings {
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn load_from_database(&self, db_pool: &PgPool) -> Result<(), sqlx::Error> {
        info!("Loading settings from database...");
        
        // Try to get the most recent settings
        if let Ok(row) = sqlx::query("SELECT settings FROM chloe_guilds_settings ORDER BY modified_at DESC LIMIT 1")
            .fetch_one(db_pool).await {
            let settings_json: Value = row.get("settings");
            let mut data = self.data.write().await;
            *data = Some(settings_json.clone());
            
            // prettify~
            let pretty_json = serde_json::to_string_pretty(&settings_json)
                .unwrap_or_else(|_| "Failed to prettify JSON".to_string());
            info!("Settings loaded successfully:\n{}", pretty_json);
        } else {
            info!("No settings found in database");
        }
        
        Ok(())
    }

    pub async fn get(&self) -> Option<Value> {
        let data = self.data.read().await;
        data.clone()
    }

    pub async fn reload_from_database(&self, db_pool: &PgPool) -> Result<(), sqlx::Error> {
        info!("Reloading settings from database...");
        self.load_from_database(db_pool).await
    }
}