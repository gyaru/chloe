use tracing::info;
use tokio_postgres::Client;
use std::sync::Arc;
use crate::settings::Settings;

pub async fn handle_update_settings(message: &str, db_client: &Arc<Client>, settings: &Settings) {
    info!("Processing updateSettings message: {}", message);
    
    let db_client = Arc::clone(db_client);
    let settings = settings.clone();
    
    tokio::spawn(async move {
        match settings.reload_from_database(&db_client).await {
            Ok(_) => {
                info!("Settings successfully reloaded from database");
            },
            Err(e) => {
                tracing::error!("Failed to reload settings: {:?}", e);
            }
        }
    });
}