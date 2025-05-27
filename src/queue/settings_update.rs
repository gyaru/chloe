use tracing::info;
use sqlx::PgPool;
use std::sync::Arc;
use crate::settings::Settings;
use crate::services::guild_service::GuildService;

pub async fn handle_update_settings(
    message: &str, 
    db_pool: &PgPool, 
    settings: &Settings,
    guild_service: &Arc<GuildService>
) {
    info!("Processing updateSettings message: {}", message);
    
    let db_pool = db_pool.clone();
    let settings = settings.clone();
    let guild_service = Arc::clone(guild_service);
    
    tokio::spawn(async move {
        match settings.reload_from_database(&db_pool).await {
            Ok(_) => {
                guild_service.clear_all_caches().await;
                info!("Settings successfully reloaded and caches cleared");
            },
            Err(e) => {
                tracing::error!("Failed to reload settings: {:?}", e);
            }
        }
    });
}