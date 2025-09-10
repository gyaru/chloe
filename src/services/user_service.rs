use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use tracing::info;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscordUserData {
    pub id: String,
    pub username: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub banner: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserAuthRequest {
    pub guild_snowflake: String,
    pub discord_data: DiscordUserData,
    pub request_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: String,
    pub snowflake_id: i64,
    pub username: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub banner: Option<String>,
    pub guild_role: Option<String>,
    pub superadmin: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserGuildInfo {
    pub guild_id: String,
    pub guild_snowflake_id: i64,
    pub guild_name: String,
    pub role: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserAuthInfo {
    pub user: UserInfo,
    pub guilds: Vec<UserGuildInfo>,
}

pub struct UserService {
    db_pool: PgPool,
}

impl UserService {
    pub fn new(db_pool: PgPool) -> Self {
        Self { db_pool }
    }

    pub async fn authenticate_user_global(
        &self,
        discord_data: DiscordUserData,
    ) -> Result<UserInfo, sqlx::Error> {
        info!(
            event = "user_global_auth_started",
            user_id = %discord_data.id,
            "Starting global user authentication"
        );

        let user_snowflake_id: i64 = discord_data
            .id
            .parse()
            .map_err(|_| sqlx::Error::Protocol("Invalid user ID format".to_string()))?;

        // Start a transaction for consistency
        let mut tx = self.db_pool.begin().await?;

        // 1. Upsert user in chloe_users with profile data
        let user_internal_id = sqlx::query_scalar::<_, String>(
            r#"
            INSERT INTO chloe_users (snowflake_id, username, global_name, avatar, banner) 
            VALUES ($1, $2, $3, $4, $5) 
            ON CONFLICT (snowflake_id) 
            DO UPDATE SET 
                username = EXCLUDED.username,
                global_name = EXCLUDED.global_name,
                avatar = EXCLUDED.avatar,
                banner = EXCLUDED.banner,
                modified_at = CURRENT_TIMESTAMP 
            RETURNING id
            "#,
        )
        .bind(user_snowflake_id)
        .bind(&discord_data.username)
        .bind(&discord_data.global_name)
        .bind(&discord_data.avatar)
        .bind(&discord_data.banner)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;

        // Get updated user info including profile data
        let user_row = sqlx::query(
            "SELECT superadmin, username, global_name, avatar, banner FROM chloe_users WHERE snowflake_id = $1"
        )
        .bind(user_snowflake_id)
        .fetch_one(&self.db_pool)
        .await?;

        let user_info = UserInfo {
            id: user_internal_id,
            snowflake_id: user_snowflake_id,
            username: user_row
                .get::<Option<String>, _>("username")
                .unwrap_or_default(),
            global_name: user_row.get("global_name"),
            avatar: user_row.get("avatar"),
            banner: user_row.get("banner"),
            guild_role: None, // No guild context
            superadmin: user_row.get("superadmin"),
        };

        info!(
            event = "user_global_auth_completed",
            user_id = %discord_data.id,
            user_internal_id = %user_info.id,
            superadmin = user_info.superadmin,
            "Global user authentication completed"
        );

        Ok(user_info)
    }

    pub async fn authenticate_user(
        &self,
        request: UserAuthRequest,
    ) -> Result<UserInfo, sqlx::Error> {
        info!(
            event = "user_auth_started",
            user_id = %request.discord_data.id,
            guild_snowflake = %request.guild_snowflake,
            request_id = %request.request_id,
            "Starting user authentication"
        );

        let user_snowflake_id: i64 = request
            .discord_data
            .id
            .parse()
            .map_err(|_| sqlx::Error::Protocol("Invalid user ID format".to_string()))?;

        let guild_snowflake_id: i64 = request
            .guild_snowflake
            .parse()
            .map_err(|_| sqlx::Error::Protocol("Invalid guild ID format".to_string()))?;

        // Start a transaction for consistency
        let mut tx = self.db_pool.begin().await?;

        // 1. Upsert user in chloe_users with profile data
        let user_internal_id = sqlx::query_scalar::<_, String>(
            r#"
            INSERT INTO chloe_users (snowflake_id, username, global_name, avatar, banner) 
            VALUES ($1, $2, $3, $4, $5) 
            ON CONFLICT (snowflake_id) 
            DO UPDATE SET 
                username = EXCLUDED.username,
                global_name = EXCLUDED.global_name,
                avatar = EXCLUDED.avatar,
                banner = EXCLUDED.banner,
                modified_at = CURRENT_TIMESTAMP 
            RETURNING id
            "#,
        )
        .bind(user_snowflake_id)
        .bind(&request.discord_data.username)
        .bind(&request.discord_data.global_name)
        .bind(&request.discord_data.avatar)
        .bind(&request.discord_data.banner)
        .fetch_one(&mut *tx)
        .await?;

        // 2. Get guild internal ID
        let guild_internal_id =
            sqlx::query_scalar::<_, String>("SELECT id FROM chloe_guilds WHERE snowflake_id = $1")
                .bind(guild_snowflake_id)
                .fetch_optional(&mut *tx)
                .await?;

        let guild_role = if let Some(guild_id) = guild_internal_id {
            // 3. Upsert user in guild (if guild exists)
            sqlx::query(
                r#"
                INSERT INTO chloe_guild_users (guild_id, user_id, role) 
                VALUES ($1, $2, 'member') 
                ON CONFLICT (guild_id, user_id) 
                DO UPDATE SET modified_at = CURRENT_TIMESTAMP
                "#,
            )
            .bind(&guild_id)
            .bind(&user_internal_id)
            .execute(&mut *tx)
            .await?;

            // 4. Get user's role in the guild
            let role = sqlx::query_scalar::<_, String>(
                "SELECT role FROM chloe_guild_users WHERE guild_id = $1 AND user_id = $2",
            )
            .bind(&guild_id)
            .bind(&user_internal_id)
            .fetch_optional(&mut *tx)
            .await?;

            role
        } else {
            info!(
                event = "guild_not_found",
                guild_snowflake = %request.guild_snowflake,
                "Guild not found in database"
            );
            None
        };

        tx.commit().await?;

        // Get updated user info including profile data
        let user_row = sqlx::query(
            "SELECT superadmin, username, global_name, avatar, banner FROM chloe_users WHERE snowflake_id = $1"
        )
        .bind(user_snowflake_id)
        .fetch_one(&self.db_pool)
        .await?;

        let user_info = UserInfo {
            id: user_internal_id,
            snowflake_id: user_snowflake_id,
            username: user_row
                .get::<Option<String>, _>("username")
                .unwrap_or_default(),
            global_name: user_row.get("global_name"),
            avatar: user_row.get("avatar"),
            banner: user_row.get("banner"),
            guild_role,
            superadmin: user_row.get("superadmin"),
        };

        info!(
            event = "user_auth_completed",
            user_id = %request.discord_data.id,
            guild_snowflake = %request.guild_snowflake,
            request_id = %request.request_id,
            guild_role = ?user_info.guild_role,
            "User authentication completed"
        );

        Ok(user_info)
    }

    pub async fn get_user(&self, user_snowflake_id: i64) -> Result<Option<UserInfo>, sqlx::Error> {
        info!(
            event = "get_user_started",
            user_snowflake_id = user_snowflake_id,
            "Getting user by snowflake ID"
        );

        let row = sqlx::query(
            r#"
            SELECT u.id, u.snowflake_id, u.username, u.global_name, u.avatar, u.banner, u.superadmin 
            FROM chloe_users u 
            WHERE u.snowflake_id = $1
            "#
        )
        .bind(user_snowflake_id)
        .fetch_optional(&self.db_pool)
        .await?;

        if let Some(row) = row {
            let user_info = UserInfo {
                id: row.get("id"),
                snowflake_id: row.get("snowflake_id"),
                username: row.get::<Option<String>, _>("username").unwrap_or_default(),
                global_name: row.get("global_name"),
                avatar: row.get("avatar"),
                banner: row.get("banner"),
                guild_role: None,
                superadmin: row.get("superadmin"),
            };

            info!(
                event = "get_user_found",
                user_snowflake_id = user_snowflake_id,
                internal_id = %user_info.id,
                "User found"
            );

            Ok(Some(user_info))
        } else {
            info!(
                event = "get_user_not_found",
                user_snowflake_id = user_snowflake_id,
                "User not found"
            );
            Ok(None)
        }
    }

    pub async fn get_users(
        &self,
        user_snowflake_ids: Vec<i64>,
    ) -> Result<HashMap<i64, UserInfo>, sqlx::Error> {
        info!(
            event = "get_users_started",
            count = user_snowflake_ids.len(),
            "Getting multiple users by snowflake IDs"
        );

        let mut result = HashMap::new();

        if user_snowflake_ids.is_empty() {
            return Ok(result);
        }

        let rows = sqlx::query(
            r#"
            SELECT u.id, u.snowflake_id, u.username, u.global_name, u.avatar, u.banner, u.superadmin 
            FROM chloe_users u 
            WHERE u.snowflake_id = ANY($1)
            "#
        )
        .bind(&user_snowflake_ids)
        .fetch_all(&self.db_pool)
        .await?;

        for row in rows {
            let snowflake_id: i64 = row.get("snowflake_id");
            let user_info = UserInfo {
                id: row.get("id"),
                snowflake_id,
                username: row.get::<Option<String>, _>("username").unwrap_or_default(),
                global_name: row.get("global_name"),
                avatar: row.get("avatar"),
                banner: row.get("banner"),
                guild_role: None,
                superadmin: row.get("superadmin"),
            };

            result.insert(snowflake_id, user_info);
        }

        info!(
            event = "get_users_completed",
            requested_count = user_snowflake_ids.len(),
            found_count = result.len(),
            "Bulk user lookup completed"
        );

        Ok(result)
    }

    pub async fn get_users_by_internal_ids(
        &self,
        user_internal_ids: Vec<String>,
    ) -> Result<HashMap<String, UserInfo>, sqlx::Error> {
        info!(
            event = "get_users_by_internal_ids_started",
            count = user_internal_ids.len(),
            "Getting multiple users by internal IDs"
        );

        let mut result = HashMap::new();

        if user_internal_ids.is_empty() {
            return Ok(result);
        }

        let rows = sqlx::query(
            r#"
            SELECT u.id, u.snowflake_id, u.username, u.global_name, u.avatar, u.banner, u.superadmin 
            FROM chloe_users u 
            WHERE u.id = ANY($1)
            "#
        )
        .bind(&user_internal_ids)
        .fetch_all(&self.db_pool)
        .await?;

        for row in rows {
            let internal_id: String = row.get("id");
            let user_info = UserInfo {
                id: internal_id.clone(),
                snowflake_id: row.get("snowflake_id"),
                username: row.get::<Option<String>, _>("username").unwrap_or_default(),
                global_name: row.get("global_name"),
                avatar: row.get("avatar"),
                banner: row.get("banner"),
                guild_role: None,
                superadmin: row.get("superadmin"),
            };

            result.insert(internal_id, user_info);
        }

        info!(
            event = "get_users_by_internal_ids_completed",
            requested_count = user_internal_ids.len(),
            found_count = result.len(),
            "Bulk user lookup by internal IDs completed"
        );

        Ok(result)
    }

    pub async fn get_user_with_guild_role(
        &self,
        user_snowflake_id: i64,
        guild_snowflake_id: i64,
    ) -> Result<Option<UserInfo>, sqlx::Error> {
        info!(
            event = "get_user_with_guild_role_started",
            user_snowflake_id = user_snowflake_id,
            guild_snowflake_id = guild_snowflake_id,
            "Getting user with guild role"
        );

        let row = sqlx::query(
            r#"
            SELECT u.id, u.snowflake_id, u.username, u.global_name, u.avatar, u.banner, u.superadmin, gu.role 
            FROM chloe_users u 
            LEFT JOIN chloe_guild_users gu ON u.id = gu.user_id 
            LEFT JOIN chloe_guilds g ON gu.guild_id = g.id 
            WHERE u.snowflake_id = $1 AND (g.snowflake_id = $2 OR g.snowflake_id IS NULL)
            "#
        )
        .bind(user_snowflake_id)
        .bind(guild_snowflake_id)
        .fetch_optional(&self.db_pool)
        .await?;

        if let Some(row) = row {
            let guild_role: Option<String> = row.get("role");
            let user_info = UserInfo {
                id: row.get("id"),
                snowflake_id: row.get("snowflake_id"),
                username: row.get::<Option<String>, _>("username").unwrap_or_default(),
                global_name: row.get("global_name"),
                avatar: row.get("avatar"),
                banner: row.get("banner"),
                guild_role,
                superadmin: row.get("superadmin"),
            };

            info!(
                event = "get_user_with_guild_role_found",
                user_snowflake_id = user_snowflake_id,
                guild_snowflake_id = guild_snowflake_id,
                guild_role = ?user_info.guild_role,
                "User with guild role found"
            );

            Ok(Some(user_info))
        } else {
            info!(
                event = "get_user_with_guild_role_not_found",
                user_snowflake_id = user_snowflake_id,
                guild_snowflake_id = guild_snowflake_id,
                "User not found or not in guild"
            );
            Ok(None)
        }
    }

    /// Get comprehensive auth info for a user: user details + all guilds with roles
    pub async fn get_user_auth_info(
        &self,
        user_snowflake_id: i64,
    ) -> Result<Option<UserAuthInfo>, sqlx::Error> {
        info!(
            event = "get_user_auth_info_started",
            user_snowflake_id = user_snowflake_id,
            "Getting comprehensive user auth info"
        );

        // First get the user
        let user_row = sqlx::query(
            r#"
            SELECT u.id, u.snowflake_id, u.username, u.global_name, u.avatar, u.banner, u.superadmin 
            FROM chloe_users u 
            WHERE u.snowflake_id = $1
            "#
        )
        .bind(user_snowflake_id)
        .fetch_optional(&self.db_pool)
        .await?;

        let Some(user_row) = user_row else {
            info!(
                event = "get_user_auth_info_not_found",
                user_snowflake_id = user_snowflake_id,
                "User not found"
            );
            return Ok(None);
        };

        let user_info = UserInfo {
            id: user_row.get("id"),
            snowflake_id: user_row.get("snowflake_id"),
            username: user_row
                .get::<Option<String>, _>("username")
                .unwrap_or_default(),
            global_name: user_row.get("global_name"),
            avatar: user_row.get("avatar"),
            banner: user_row.get("banner"),
            guild_role: None, // Not applicable for this method
            superadmin: user_row.get("superadmin"),
        };

        // Get all guilds the user is in
        let guild_rows = sqlx::query(
            r#"
            SELECT g.id, g.snowflake_id, g.name, gu.role 
            FROM chloe_guilds g
            INNER JOIN chloe_guild_users gu ON g.id = gu.guild_id
            WHERE gu.user_id = $1
            ORDER BY g.name
            "#,
        )
        .bind(&user_info.id)
        .fetch_all(&self.db_pool)
        .await?;

        let mut guilds = Vec::new();
        for row in guild_rows {
            guilds.push(UserGuildInfo {
                guild_id: row.get("id"),
                guild_snowflake_id: row.get("snowflake_id"),
                guild_name: row.get("name"),
                role: row.get("role"),
            });
        }

        let auth_info = UserAuthInfo {
            user: user_info,
            guilds,
        };

        info!(
            event = "get_user_auth_info_completed",
            user_snowflake_id = user_snowflake_id,
            guild_count = auth_info.guilds.len(),
            superadmin = auth_info.user.superadmin,
            "User auth info retrieved successfully"
        );

        Ok(Some(auth_info))
    }
}
