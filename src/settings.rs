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
        
        if let Ok(row) = sqlx::query(
            "SELECT p.content as prompt FROM chloe_settings s 
             JOIN chloe_prompts p ON s.prompt_id = p.id 
             WHERE s.id = 1"
        )
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

    pub async fn create_new_prompt_version(&self, db_pool: &PgPool, content: &str, created_by: Option<&str>) -> Result<String, sqlx::Error> {
        info!(
            event = "new_prompt_version_creating",
            content_length = content.len(),
            created_by = created_by.unwrap_or("unknown"),
            "Creating new prompt version"
        );

        // Get the next version number
        let next_version: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM chloe_prompts"
        )
        .fetch_one(db_pool)
        .await?;

        // Create new prompt version
        let prompt_id = sqlx::query_scalar::<_, String>(
            "INSERT INTO chloe_prompts (version, content, created_by, is_active) VALUES ($1, $2, $3, false) RETURNING id"
        )
        .bind(next_version)
        .bind(content)
        .bind(created_by.unwrap_or("unknown"))
        .fetch_one(db_pool)
        .await?;

        info!(
            event = "new_prompt_version_created",
            prompt_id = %prompt_id,
            version = next_version,
            created_by = created_by.unwrap_or("unknown"),
            "New prompt version created"
        );

        Ok(prompt_id)
    }

    pub async fn activate_prompt_version(&self, db_pool: &PgPool, prompt_id: &str) -> Result<(), sqlx::Error> {
        info!(
            event = "prompt_version_activating",
            prompt_id = %prompt_id,
            "Activating prompt version"
        );

        // Start transaction to ensure consistency
        let mut tx = db_pool.begin().await?;

        // Deactivate all other prompts
        sqlx::query("UPDATE chloe_prompts SET is_active = false")
            .execute(&mut *tx)
            .await?;

        // Activate the specified prompt
        sqlx::query("UPDATE chloe_prompts SET is_active = true WHERE id = $1")
            .bind(prompt_id)
            .execute(&mut *tx)
            .await?;

        // Update settings to reference the new prompt
        sqlx::query("UPDATE chloe_settings SET prompt_id = $1, modified_at = CURRENT_TIMESTAMP WHERE id = 1")
            .bind(prompt_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        // Reload settings
        self.reload_global_settings(db_pool).await?;

        info!(
            event = "prompt_version_activated",
            prompt_id = %prompt_id,
            "Prompt version activated and settings reloaded"
        );

        Ok(())
    }
}
