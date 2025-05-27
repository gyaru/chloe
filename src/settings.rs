use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_postgres::{Client, Error as PgError};
use serde_json::Value;
use tracing::{info, error};

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

    pub async fn load_from_database(&self, db_client: &Client) -> Result<(), PgError> {
        info!("Loading settings from database...");
        
        let query = "SELECT settings FROM chloe ORDER BY date_modified DESC LIMIT 1";
        let rows = db_client.query(query, &[]).await?;
        
        if let Some(row) = rows.first() {
            let settings_json: Value = row.get(0);
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

    pub async fn reload_from_database(&self, db_client: &Client) -> Result<(), PgError> {
        info!("Reloading settings from database...");
        self.load_from_database(db_client).await
    }
}