use serde_json::json;
use serenity::model::prelude::*;
use sqlx::{PgPool, Row};
use tracing::{error, info};

pub async fn initialize_database(db_pool: &PgPool) -> Result<(), sqlx::Error> {
    info!("Initializing database schema...");

    // create chloe_users table
    let create_users_table = r#"
        CREATE TABLE IF NOT EXISTS chloe_users (
            id VARCHAR(255) PRIMARY KEY DEFAULT gen_random_uuid()::text,
            snowflake_id BIGINT UNIQUE NOT NULL,
            username VARCHAR(255),
            global_name VARCHAR(255),
            avatar VARCHAR(255),
            banner VARCHAR(255),
            superadmin BOOLEAN NOT NULL DEFAULT false,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            modified_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )
    "#;

    // create chloe_guilds table
    let create_guilds_table = r#"
        CREATE TABLE IF NOT EXISTS chloe_guilds (
            id VARCHAR(255) PRIMARY KEY DEFAULT gen_random_uuid()::text,
            snowflake_id BIGINT UNIQUE NOT NULL,
            name VARCHAR(255) NOT NULL,
            owner_id VARCHAR(255) REFERENCES chloe_users(id),
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            modified_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )
    "#;

    // create chloe_guilds_settings table
    let create_settings_table = r#"
        CREATE TABLE IF NOT EXISTS chloe_guilds_settings (
            id VARCHAR(255) PRIMARY KEY DEFAULT gen_random_uuid()::text,
            guild_id VARCHAR(255) UNIQUE REFERENCES chloe_guilds(id),
            settings JSON NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            modified_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )
    "#;

    // create chloe_guild_users bridge table for many-to-many relationship
    let create_guild_users_table = r#"
        CREATE TABLE IF NOT EXISTS chloe_guild_users (
            id VARCHAR(255) PRIMARY KEY DEFAULT gen_random_uuid()::text,
            guild_id VARCHAR(255) REFERENCES chloe_guilds(id),
            user_id VARCHAR(255) REFERENCES chloe_users(id),
            role VARCHAR(255) NOT NULL DEFAULT 'member',
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            modified_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(guild_id, user_id)
        )
    "#;

    // create chloe_prompts table for versioned prompts
    let create_prompts_table = r#"
        CREATE TABLE IF NOT EXISTS chloe_prompts (
            id VARCHAR(255) PRIMARY KEY DEFAULT gen_random_uuid()::text,
            version INTEGER NOT NULL,
            content TEXT NOT NULL,
            created_by VARCHAR(255),
            is_active BOOLEAN NOT NULL DEFAULT false,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            modified_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(version)
        )
    "#;

    // create chloe_settings table for global bot settings
    let create_global_settings_table = r#"
        CREATE TABLE IF NOT EXISTS chloe_settings (
            id INTEGER PRIMARY KEY DEFAULT 1,
            prompt_id VARCHAR(255) REFERENCES chloe_prompts(id),
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            modified_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            CONSTRAINT single_row CHECK (id = 1)
        )
    "#;

    // execute table creation
    sqlx::query(create_users_table).execute(db_pool).await?;
    info!("created/verified chloe_users table");

    // add new columns if they don't exist (migrations)
    let add_user_columns = r#"
        ALTER TABLE chloe_users 
        ADD COLUMN IF NOT EXISTS username VARCHAR(255),
        ADD COLUMN IF NOT EXISTS global_name VARCHAR(255),
        ADD COLUMN IF NOT EXISTS avatar VARCHAR(255),
        ADD COLUMN IF NOT EXISTS banner VARCHAR(255),
        ADD COLUMN IF NOT EXISTS superadmin BOOLEAN NOT NULL DEFAULT false
    "#;
    sqlx::query(add_user_columns).execute(db_pool).await?;
    info!("ensured user profile columns exist in chloe_users table");

    sqlx::query(create_guilds_table).execute(db_pool).await?;
    info!("created/verified chloe_guilds table");

    sqlx::query(create_settings_table).execute(db_pool).await?;
    info!("created/verified chloe_guilds_settings table");

    sqlx::query(create_guild_users_table)
        .execute(db_pool)
        .await?;
    info!("created/verified chloe_guild_users table");

    sqlx::query(create_prompts_table).execute(db_pool).await?;
    info!("created/verified chloe_prompts table");

    sqlx::query(create_global_settings_table)
        .execute(db_pool)
        .await?;
    info!("created/verified chloe_settings table");

    // create performance indexes
    create_performance_indexes(db_pool).await?;

    info!("Database schema initialization complete");
    Ok(())
}

async fn create_performance_indexes(db_pool: &PgPool) -> Result<(), sqlx::Error> {
    info!("creating performance indexes...");
    sqlx::query("CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_guilds_snowflake ON chloe_guilds(snowflake_id)")
        .execute(db_pool).await?;
    sqlx::query(
        "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_users_snowflake ON chloe_users(snowflake_id)",
    )
    .execute(db_pool)
    .await?;
    sqlx::query("CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_guild_users_lookup ON chloe_guild_users(guild_id, user_id)")
        .execute(db_pool).await?;
    sqlx::query("CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_settings_guild ON chloe_guilds_settings(guild_id)")
        .execute(db_pool).await?;
    sqlx::query("CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_guilds_settings_covering ON chloe_guilds_settings(guild_id) INCLUDE (settings)")
        .execute(db_pool).await?;
    info!("Performance indexes created successfully");
    Ok(())
}

pub async fn sync_guilds(
    db_pool: &PgPool,
    guilds: &[GuildId],
    ctx: &serenity::prelude::Context,
) -> Result<(), sqlx::Error> {
    info!("Synchronizing {} guilds to database...", guilds.len());

    for guild_id in guilds {
        if let Ok(guild) = guild_id.to_partial_guild(&ctx.http).await {
            if let Err(e) = sqlx::query(
                "INSERT INTO chloe_users (snowflake_id) VALUES ($1) ON CONFLICT (snowflake_id) DO NOTHING"
            )
            .bind(guild.owner_id.get() as i64)
            .execute(db_pool).await {
                error!("Failed to insert/update user {}: {:?}", guild.owner_id, e);
                continue;
            }

            if let Ok(user_row) = sqlx::query("SELECT id FROM chloe_users WHERE snowflake_id = $1")
                .bind(guild.owner_id.get() as i64)
                .fetch_one(db_pool)
                .await
            {
                let owner_internal_id: String = user_row.get("id");

                match sqlx::query(
                    r#"
                    INSERT INTO chloe_guilds (snowflake_id, name, owner_id)
                    VALUES ($1, $2, $3)
                    ON CONFLICT (snowflake_id) 
                    DO UPDATE SET 
                        name = EXCLUDED.name,
                        owner_id = EXCLUDED.owner_id,
                        modified_at = CURRENT_TIMESTAMP
                    "#,
                )
                .bind(guild_id.get() as i64)
                .bind(&guild.name)
                .bind(&owner_internal_id)
                .execute(db_pool)
                .await
                {
                    Ok(_) => {
                        info!("Synced guild: {} ({})", guild.name, guild_id);

                        // get the guild's internal ID
                        if let Ok(guild_row) =
                            sqlx::query("SELECT id FROM chloe_guilds WHERE snowflake_id = $1")
                                .bind(guild_id.get() as i64)
                                .fetch_one(db_pool)
                                .await
                        {
                            let guild_internal_id: String = guild_row.get("id");

                            if let Err(e) = sqlx::query(
                                r#"
                                INSERT INTO chloe_guild_users (guild_id, user_id, role)
                                VALUES ($1, $2, 'admin')
                                ON CONFLICT (guild_id, user_id) 
                                DO UPDATE SET 
                                    role = EXCLUDED.role,
                                    modified_at = CURRENT_TIMESTAMP
                                "#,
                            )
                            .bind(&guild_internal_id)
                            .bind(&owner_internal_id)
                            .execute(db_pool)
                            .await
                            {
                                error!(
                                    "Failed to add owner as admin to guild {}: {:?}",
                                    guild_id, e
                                );
                            } else {
                                info!("Added guild owner as admin for guild: {}", guild.name);
                            }

                            // create default settings if they don't exist
                            if let Err(e) =
                                create_default_settings(db_pool, &guild_internal_id).await
                            {
                                error!(
                                    "Failed to create default settings for guild {}: {:?}",
                                    guild_id, e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to sync guild {}: {:?}", guild_id, e);
                    }
                }
            }
        } else {
            error!("Failed to get guild info for {}", guild_id);
        }
    }

    info!("Guild synchronization complete");
    Ok(())
}

pub async fn ensure_global_settings(db_pool: &PgPool) -> Result<(), sqlx::Error> {
    let existing_settings = sqlx::query("SELECT id FROM chloe_settings WHERE id = 1")
        .fetch_optional(db_pool)
        .await?;

    if existing_settings.is_none() {
        // First, create a default prompt
        let prompt_id = sqlx::query_scalar::<_, String>(
            "INSERT INTO chloe_prompts (version, content, created_by, is_active) VALUES (1, $1, $2, true) RETURNING id"
        )
        .bind("You're Chloe, a discord bot.")
        .bind("system")
        .fetch_one(db_pool)
        .await?;

        // Then create the settings row referencing the prompt
        sqlx::query("INSERT INTO chloe_settings (id, prompt_id) VALUES (1, $1)")
            .bind(&prompt_id)
            .execute(db_pool)
            .await?;

        info!("Created default global settings with prompt version 1");
    }

    Ok(())
}

pub async fn create_default_settings(
    db_pool: &PgPool,
    guild_internal_id: &str,
) -> Result<(), sqlx::Error> {
    let default_settings = json!({
        "ping_reply": false,
        "llm": false
    });

    let existing_settings = sqlx::query("SELECT id FROM chloe_guilds_settings WHERE guild_id = $1")
        .bind(guild_internal_id)
        .fetch_optional(db_pool)
        .await?;

    if existing_settings.is_none() {
        sqlx::query("INSERT INTO chloe_guilds_settings (guild_id, settings) VALUES ($1, $2)")
            .bind(guild_internal_id)
            .bind(&default_settings)
            .execute(db_pool)
            .await?;

        info!("Created default settings for guild");
    } else {
        info!("Settings already exist for guild, skipping default creation");
    }

    Ok(())
}
