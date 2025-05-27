use serde_json::Value;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

#[derive(Clone)]
pub struct GuildService {
    db_pool: PgPool,
    settings_cache: Arc<RwLock<HashMap<i64, Value>>>,
    role_cache: Arc<RwLock<HashMap<(i64, i64), String>>>, // (guild_id, user_id) -> role
}

impl GuildService {
    pub fn new(db_pool: PgPool) -> Self {
        Self {
            db_pool,
            settings_cache: Arc::new(RwLock::new(HashMap::new())),
            role_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_user_role(&self, guild_id: i64, user_id: i64) -> Option<String> {
        let cache_key = (guild_id, user_id);

        {
            let cache = self.role_cache.read().await;
            if let Some(role) = cache.get(&cache_key) {
                return Some(role.clone());
            }
        }

        if let Ok(role) = self.load_user_role_from_db(guild_id, user_id).await {
            let mut cache = self.role_cache.write().await;
            cache.insert(cache_key, role.clone());
            Some(role)
        } else {
            None
        }
    }

    pub async fn is_user_admin(&self, guild_id: i64, user_id: i64) -> bool {
        self.get_user_role(guild_id, user_id)
            .await
            .map(|role| role == "admin")
            .unwrap_or(false)
    }

    pub async fn get_guild_setting(&self, guild_id: i64, key: &str) -> Option<Value> {
        // Check cache first
        {
            let cache = self.settings_cache.read().await;
            if let Some(settings) = cache.get(&guild_id) {
                return settings.get(key).cloned();
            }
        }

        if let Ok(settings) = self.load_guild_settings_from_db(guild_id).await {
            let mut cache = self.settings_cache.write().await;
            cache.insert(guild_id, settings.clone());
            settings.get(key).cloned()
        } else {
            None
        }
    }

    pub async fn clear_all_caches(&self) {
        let mut settings_cache = self.settings_cache.write().await;
        let mut role_cache = self.role_cache.write().await;
        settings_cache.clear();
        role_cache.clear();
        info!("Cleared all caches");
    }

    async fn load_user_role_from_db(
        &self,
        guild_id: i64,
        user_id: i64,
    ) -> Result<String, sqlx::Error> {
        let row = sqlx::query(
            "SELECT gu.role FROM chloe_guild_users gu 
             JOIN chloe_guilds g ON gu.guild_id = g.id 
             JOIN chloe_users u ON gu.user_id = u.id 
             WHERE g.snowflake_id = $1 AND u.snowflake_id = $2",
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_one(&self.db_pool)
        .await?;

        Ok(row.get("role"))
    }

    async fn load_guild_settings_from_db(&self, guild_id: i64) -> Result<Value, sqlx::Error> {
        let row = sqlx::query(
            "SELECT gs.settings FROM chloe_guilds_settings gs 
             JOIN chloe_guilds g ON gs.guild_id = g.id 
             WHERE g.snowflake_id = $1",
        )
        .bind(guild_id)
        .fetch_one(&self.db_pool)
        .await?;

        Ok(row.get("settings"))
    }
}
