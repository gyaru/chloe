use anyhow::Result;
use serenity::client::ClientBuilder;
use serenity::model::gateway::GatewayIntents;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};

mod commands;
mod database;
mod queue;
mod reactions;
mod redis_client;
mod schema;
mod services;
mod settings;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

pub struct Data {
    redis_client: redis::Client,
    db_pool: PgPool,
    settings: settings::Settings,
    guild_service: Arc<services::guild_service::GuildService>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("chloe=info".parse()?),
        )
        .init();

    info!("starting chloe 💅💄");

    let redis_url = std::env::var("REDIS_URL").expect("Expected REDIS_URL in environment");
    let redis_client = redis::Client::open(redis_url)?;

    let postgres_url = std::env::var("POSTGRES_URL").expect("Expected POSTGRES_URL in environment");

    // do I really need to pool?
    let db_pool = PgPoolOptions::new()
        .max_connections(10)
        .min_connections(2)
        .acquire_timeout(Duration::from_secs(3))
        .idle_timeout(Duration::from_secs(300))
        .max_lifetime(Duration::from_secs(1800))
        .connect(&postgres_url)
        .await?;

    info!("connected to postgres database");

    info!("connected to redis");

    // Initialize services
    let app_settings = settings::Settings::new();
    let guild_service = Arc::new(services::guild_service::GuildService::new(db_pool.clone()));

    let redis_client_for_framework = redis_client.clone();
    let db_pool_for_framework = db_pool.clone();
    let settings_for_framework = app_settings.clone();
    let guild_service_for_framework = Arc::clone(&guild_service);

    let queue_listener = queue::QueueListener::new(
        redis_client.clone(),
        db_pool.clone(),
        app_settings.clone(),
        Arc::clone(&guild_service),
    );
    tokio::spawn(async move {
        queue_listener.start_listening().await;
    });

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![commands::ping::ping(), commands::status::status()],
            ..Default::default()
        })
        .setup(move |ctx, _ready, framework| {
            let redis_client = redis_client_for_framework;
            let db_pool = db_pool_for_framework;
            let settings = settings_for_framework;
            let guild_service = guild_service_for_framework;

            Box::pin(async move {
                if let Err(e) = schema::initialize_database(&db_pool).await {
                    error!("Failed to initialize database: {:?}", e);
                }

                if let Err(e) = schema::ensure_global_settings(&db_pool).await {
                    error!("Failed to ensure global settings: {:?}", e);
                }

                let current_guilds: Vec<_> = ctx.cache.guilds().iter().cloned().collect();
                if let Err(e) = schema::sync_guilds(&db_pool, &current_guilds, ctx).await {
                    error!("Failed to sync guilds: {:?}", e);
                } else {
                    info!(
                        "Successfully synced {} guilds to database",
                        current_guilds.len()
                    );
                }

                if let Err(e) = settings.load_from_database(&db_pool).await {
                    error!("Failed to load settings: {:?}", e);
                }

                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                info!("commands registered");
                Ok(Data {
                    redis_client,
                    db_pool,
                    settings,
                    guild_service,
                })
            })
        })
        .build();

    let token = std::env::var("DISCORD_TOKEN").expect("Expected DISCORD_TOKEN in environment");

    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;

    let client = ClientBuilder::new(token, intents)
        .framework(framework)
        .event_handler(reactions::llm_handler::LLMHandler {
            guild_service: Arc::clone(&guild_service),
        })
        .await;

    client?.start().await?;

    Ok(())
}
