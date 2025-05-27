use anyhow::Result;
use tokio_postgres::{NoTls};
use serenity::{prelude::*, Client};
use serenity::client::ClientBuilder;
use serenity::model::gateway::GatewayIntents;
use tracing::{info, error};
use std::sync::Arc;

mod commands;
mod reactions;
mod redis_client;
mod queue;
mod database;
mod settings;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

pub struct Data {
    redis_client: redis::Client,
    postgres_client: Arc<tokio_postgres::Client>,
    settings: settings::Settings,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("chloe=info".parse()?))
        .init();

    info!("starting chloe 💅💄");

    let redis_url = std::env::var("REDIS_URL")
        .expect("Expected REDIS_URL in environment");
    let redis_client = redis::Client::open(redis_url)?;

    let postgres_url = std::env::var("POSTGRES_URL")
        .expect("Expected POSTGRES_URL in environment");
    
    let (postgres_client, connection) = tokio_postgres::connect(&postgres_url, NoTls).await?;
    let postgres_client = Arc::new(postgres_client);
    
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            error!("PostgreSQL connection error: {}", e);
        }
    });
    
    info!("connected to postgres database");
    
    info!("connected to redis");

    let app_settings = settings::Settings::new();
    
    let redis_client_for_framework = redis_client.clone();
    let postgres_client_for_framework = Arc::clone(&postgres_client);
    let settings_for_framework = app_settings.clone();
    
    let queue_listener = queue::QueueListener::new(redis_client.clone(), Arc::clone(&postgres_client), app_settings.clone());
    tokio::spawn(async move {
        queue_listener.start_listening().await;
    });
    
    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![commands::ping::ping()],
            ..Default::default()
        })
        .setup(move |ctx, _ready, framework| {
            let redis_client = redis_client_for_framework;
            let postgres_client = postgres_client_for_framework;
            let settings = settings_for_framework;
            
            Box::pin(async move {
                if let Err(e) = settings.load_from_database(&postgres_client).await {
                    error!("Failed to load settings: {:?}", e);
                }
                
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                info!("commands registered");
                Ok(Data { redis_client, postgres_client, settings })
            })
        })
        .build();

    let token = std::env::var("DISCORD_TOKEN")
        .expect("Expected DISCORD_TOKEN in environment");
    
    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;

    let client = ClientBuilder::new(token, intents)
        .framework(framework)
        .event_handler(reactions::combined_handler::CombinedHandler)
        .await;

    client?.start().await?;
    
    Ok(())
}