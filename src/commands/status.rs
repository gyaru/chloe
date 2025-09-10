use crate::{Context, Error};
use poise::serenity_prelude as serenity;
use sqlx::Row;
use std::time::{Duration, SystemTime};
use sysinfo::{Pid, System};

/// Display comprehensive bot status and metrics
#[poise::command(slash_command)]
pub async fn status(ctx: Context<'_>) -> Result<(), Error> {
    let start_time = SystemTime::now();

    // Defer the response since we'll be collecting a lot of metrics
    ctx.defer().await?;

    // Collect all metrics concurrently
    let (runtime_metrics, system_info, db_health, redis_health) = tokio::join!(
        collect_runtime_metrics(),
        collect_system_metrics(),
        check_database_health(&ctx.data().db_pool),
        check_redis_health(&ctx.data().redis_client)
    );

    let collection_time = start_time.elapsed().unwrap_or(Duration::ZERO);

    let embed = serenity::CreateEmbed::new()
        .title("chloe's vibe check 💅")
        .color(0x00ff00)
        .thumbnail("https://cdn.discordapp.com/emojis/123456789.png") // Placeholder
        .timestamp(serenity::Timestamp::now())
        .field("runtime", runtime_metrics, true)
        .field("resources", system_info, true)
        .field("database", db_health, true)
        .field("cache", redis_health, true)
        .field("guild info", format_guild_info(ctx), true)
        .field(
            "collection time",
            format!("{}ms", collection_time.as_millis()),
            true,
        )
        .footer(serenity::CreateEmbedFooter::new(
            "chloe v0.1.0 • made with sparkle and good vibes! ✨",
        ));

    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}

async fn collect_runtime_metrics() -> String {
    let metrics = tokio::runtime::Handle::current().metrics();

    format!("**workers:** {}\n**runtime:** tokio", metrics.num_workers())
}

async fn collect_system_metrics() -> String {
    let mut system = System::new_all();
    system.refresh_all();

    let current_pid = sysinfo::get_current_pid().ok();
    let process_info = current_pid
        .and_then(|pid| system.process(Pid::from_u32(pid.as_u32())))
        .map(|process| {
            format!(
                "**chloe memory:** {:.1} MB\n**chloe cpu:** {:.1}%\n",
                process.memory() as f64 / 1024.0 / 1024.0,
                process.cpu_usage()
            )
        })
        .unwrap_or_else(|| "**Process Info:** Unavailable\n".to_string());

    let load_avg = System::load_average();

    format!(
        "{}**system memory:** {:.1} GB / {:.1} GB\n**system cpu:** {:.1}%\n**load avg:** {:.2}",
        process_info,
        system.used_memory() as f64 / 1024.0 / 1024.0 / 1024.0,
        system.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0,
        system.global_cpu_usage(),
        load_avg.one
    )
}

async fn check_database_health(db_pool: &sqlx::PgPool) -> String {
    let start = SystemTime::now();

    let test_query = sqlx::query("SELECT 1 as test").fetch_one(db_pool).await;

    let latency = start.elapsed().unwrap_or(Duration::ZERO);

    match test_query {
        Ok(_) => {
            let pool_info = format!(
                "**latency:** {}ms\n**pool size:** {}/{}",
                latency.as_millis(),
                db_pool.size(),
                db_pool.options().get_max_connections()
            );

            // Try to get some basic stats
            let stats_query = sqlx::query("SELECT count(*) as guilds FROM chloe_guilds")
                .fetch_one(db_pool)
                .await;

            if let Ok(row) = stats_query {
                let guild_count: i64 = row.get("guilds");
                format!("{}\n**guilds:** {}", pool_info, guild_count)
            } else {
                pool_info
            }
        }
        Err(e) => format!(
            "**status:** 🔴 error\n**error:** {}",
            e.to_string().chars().take(50).collect::<String>()
        ),
    }
}

async fn check_redis_health(redis_client: &redis::Client) -> String {
    let start = SystemTime::now();

    let mut con = match redis_client.get_connection() {
        Ok(con) => con,
        Err(e) => {
            return format!(
                "**status:** 🔴 connection failed\n**error:** {}",
                e.to_string().chars().take(50).collect::<String>()
            );
        }
    };

    let ping_result: Result<String, redis::RedisError> = redis::cmd("PING").query(&mut con);

    let latency = start.elapsed().unwrap_or(Duration::ZERO);

    match ping_result {
        Ok(_) => {
            format!("**latency:** {}ms", latency.as_millis())
        }
        Err(e) => format!(
            "**status:** 🔴 error\n**Error:** {}",
            e.to_string().chars().take(50).collect::<String>()
        ),
    }
}

fn format_guild_info(ctx: Context<'_>) -> String {
    let member_count = ctx
        .guild()
        .map(|g| g.member_count.to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let cache = &ctx.serenity_context().cache;
    let guild_count = cache.guild_count();
    let user_count = cache.user_count();

    format!(
        "**members:** {}\n**cached guilds:** {}\n**cached users:** {}",
        member_count, guild_count, user_count
    )
}
