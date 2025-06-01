use crate::settings::Settings;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{info, error};

pub async fn handle_update_prompt(
    message: &str, 
    settings: Arc<Settings>, 
    db_pool: &PgPool
) {
    info!("Processing updatePrompt message: {}", message);

    match serde_json::from_str::<Value>(message) {
        Ok(parsed_message) => {
            if let Some(action) = parsed_message.get("action") {
                match action.as_str() {
                    Some("prompt_create") => {
                        if let Some(content) = parsed_message.get("content").and_then(|v| v.as_str()) {
                            let created_by = parsed_message.get("created_by").and_then(|v| v.as_str());
                            handle_create_and_activate_prompt(&settings, db_pool, content, created_by).await;
                        } else {
                            error!("Missing 'content' field for prompt_create action");
                        }
                    }
                    Some("prompt_activate") => {
                        if let Some(prompt_id) = parsed_message.get("prompt_id").and_then(|v| v.as_str()) {
                            handle_activate_prompt_version(&settings, db_pool, prompt_id).await;
                        } else {
                            error!("Missing 'prompt_id' field for prompt_activate action");
                        }
                    }
                    _ => {
                        error!("Unknown action in updatePrompt message: {:?}", action);
                    }
                }
            } else {
                error!("Missing 'action' field in updatePrompt message");
            }
        }
        Err(e) => {
            error!("Failed to parse updatePrompt message as JSON: {:?}", e);
        }
    }

    info!("updatePrompt message processing complete");
}


async fn handle_activate_prompt_version(
    settings: &Settings, 
    db_pool: &PgPool, 
    prompt_id: &str
) {
    match settings.activate_prompt_version(db_pool, prompt_id).await {
        Ok(()) => {
            info!("Successfully activated prompt version: {}", prompt_id);
        }
        Err(e) => {
            error!("Failed to activate prompt version {}: {:?}", prompt_id, e);
        }
    }
}

async fn handle_create_and_activate_prompt(
    settings: &Settings, 
    db_pool: &PgPool, 
    content: &str,
    created_by: Option<&str>
) {
    match settings.create_new_prompt_version(db_pool, content, created_by).await {
        Ok(prompt_id) => {
            info!("Successfully created new prompt version: {}", prompt_id);
            
            match settings.activate_prompt_version(db_pool, &prompt_id).await {
                Ok(()) => {
                    info!("Successfully activated new prompt version: {}", prompt_id);
                }
                Err(e) => {
                    error!("Failed to activate newly created prompt version {}: {:?}", prompt_id, e);
                }
            }
        }
        Err(e) => {
            error!("Failed to create new prompt version: {:?}", e);
        }
    }
}
