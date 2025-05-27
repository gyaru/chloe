use crate::services::guild_service::GuildService;
use crate::settings::Settings;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::time::{Duration, timeout};
use tracing::{error, info, warn};

pub async fn handle_update_settings(
    message: &str,
    db_pool: &PgPool,
    settings: &Settings,
    guild_service: &Arc<GuildService>,
) {
    info!(
        event = "settings_update_started",
        message = %message,
        "Processing updateSettings message"
    );

    let settings_reload = timeout(Duration::from_secs(10), async {
        match settings.reload_from_database(db_pool).await {
            Ok(_) => {
                guild_service.clear_all_caches().await;
                info!(
                    event = "settings_update_completed",
                    "All settings successfully reloaded and caches cleared"
                );
                Ok(())
            }
            Err(e) => {
                error!(
                    event = "settings_update_failed",
                    error = ?e,
                    "Failed to reload settings"
                );
                Err(e)
            }
        }
    })
    .await;

    match settings_reload {
        Ok(_) => {
            info!(
                event = "settings_update_finished",
                "Settings update completed successfully"
            );
        }
        Err(_) => {
            warn!(
                event = "settings_update_timeout",
                "Settings update timed out after 10 seconds - possible deadlock"
            );
        }
    }
}
