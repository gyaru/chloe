use crate::services::user_service::{UserService, UserAuthRequest, DiscordUserData};
use redis::{Client, AsyncCommands};
use serde_json::{Value, json};
use std::sync::Arc;
use tracing::{info, error};

pub async fn handle_user_operations(
    message: &str,
    user_service: Arc<UserService>,
    redis_client: &Client,
) {
    info!("Processing user operation message: {}", message);

    match serde_json::from_str::<Value>(message) {
        Ok(parsed_message) => {
            if let Some(action) = parsed_message.get("action") {
                match action.as_str() {
                    Some("auth_user") => {
                        handle_auth_user(&parsed_message, &user_service, redis_client).await;
                    }
                    Some("get_user") => {
                        handle_get_user(&parsed_message, &user_service, redis_client).await;
                    }
                    Some("get_users") => {
                        handle_get_users(&parsed_message, &user_service, redis_client).await;
                    }
                    Some("get_user_auth") => {
                        handle_get_user_auth(&parsed_message, &user_service, redis_client).await;
                    }
                    _ => {
                        error!("Unknown action in user operations message: {:?}", action);
                    }
                }
            } else {
                error!("Missing 'action' field in user operations message");
            }
        }
        Err(e) => {
            error!("Failed to parse user operations message as JSON: {:?}", e);
        }
    }

    info!("User operations message processing complete");
}

async fn handle_auth_user(parsed_message: &Value, user_service: &UserService, redis_client: &Client) {
    // guild_snowflake is now optional for auth_user
    let guild_snowflake = parsed_message.get("guild_snowflake")
        .and_then(|v| v.as_str())
        .unwrap_or("0"); // Use "0" as placeholder when no guild specified

    let request_id = match parsed_message.get("request_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            error!("Missing 'request_id' field for auth_user action");
            return;
        }
    };

    let discord_data = match parsed_message.get("discord_data") {
        Some(data) => {
            match serde_json::from_value::<DiscordUserData>(data.clone()) {
                Ok(user_data) => user_data,
                Err(e) => {
                    error!("Failed to parse discord_data: {:?}", e);
                    return;
                }
            }
        }
        None => {
            error!("Missing 'discord_data' field for auth_user action");
            return;
        }
    };

    let response = if guild_snowflake == "0" {
        // Global authentication without guild context
        match user_service.authenticate_user_global(discord_data).await {
            Ok(user_info) => {
                info!(
                    event = "global_auth_user_success",
                    request_id = %request_id,
                    user_internal_id = %user_info.id,
                    "Global user authentication successful"
                );
                
                json!({
                    "success": true,
                    "request_id": request_id,
                    "data": {
                        "user_id": user_info.id,
                        "snowflake_id": user_info.snowflake_id.to_string(),
                        "username": user_info.username,
                        "global_name": user_info.global_name,
                        "avatar": user_info.avatar,
                        "banner": user_info.banner,
                        "guild_role": user_info.guild_role,
                        "superadmin": user_info.superadmin
                    }
                })
            }
            Err(e) => {
                error!(
                    event = "global_auth_user_failed",
                    request_id = %request_id,
                    error = ?e,
                    "Global user authentication failed"
                );
                
                json!({
                    "success": false,
                    "request_id": request_id,
                    "error": format!("Global authentication failed: {:?}", e)
                })
            }
        }
    } else {
        // Guild-specific authentication
        let auth_request = UserAuthRequest {
            guild_snowflake: guild_snowflake.to_string(),
            discord_data,
            request_id: request_id.to_string(),
        };

        match user_service.authenticate_user(auth_request).await {
            Ok(user_info) => {
                info!(
                    event = "auth_user_success",
                    request_id = %request_id,
                    user_internal_id = %user_info.id,
                    "User authentication successful"
                );
                
                json!({
                    "success": true,
                    "request_id": request_id,
                    "data": {
                        "user_id": user_info.id,
                        "snowflake_id": user_info.snowflake_id.to_string(),
                        "username": user_info.username,
                        "global_name": user_info.global_name,
                        "avatar": user_info.avatar,
                        "banner": user_info.banner,
                        "guild_role": user_info.guild_role,
                        "superadmin": user_info.superadmin
                    }
                })
            }
            Err(e) => {
                error!(
                    event = "auth_user_failed",
                    request_id = %request_id,
                    error = ?e,
                    "User authentication failed"
                );
                
                json!({
                    "success": false,
                    "request_id": request_id,
                    "error": format!("Authentication failed: {:?}", e)
                })
            }
        }
    };

    // Send response back via Redis
    send_response(redis_client, &response).await;
}

async fn handle_get_user(parsed_message: &Value, user_service: &UserService, redis_client: &Client) {
    let user_snowflake_str = match parsed_message.get("snowflake_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            error!("Missing 'user_snowflake_id' field for get_user action");
            return;
        }
    };

    let user_snowflake_id: i64 = match user_snowflake_str.parse() {
        Ok(id) => id,
        Err(_) => {
            error!("Invalid user_snowflake_id format: {}", user_snowflake_str);
            return;
        }
    };

    let request_id = parsed_message.get("request_id").and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let response = match user_service.get_user(user_snowflake_id).await {
        Ok(Some(user_info)) => {
            info!(
                event = "get_user_success",
                request_id = %request_id,
                user_snowflake_id = user_snowflake_id,
                user_internal_id = %user_info.id,
                "User lookup successful"
            );
            
            json!({
                "success": true,
                "request_id": request_id,
                "data": {
                    "user_id": user_info.id,
                    "snowflake_id": user_info.snowflake_id.to_string(),
                    "username": user_info.username,
                    "global_name": user_info.global_name,
                    "avatar": user_info.avatar,
                    "banner": user_info.banner,
                    "guild_role": user_info.guild_role,
                    "superadmin": user_info.superadmin
                }
            })
        }
        Ok(None) => {
            info!(
                event = "get_user_not_found",
                request_id = %request_id,
                user_snowflake_id = user_snowflake_id,
                "User not found"
            );
            
            json!({
                "success": false,
                "request_id": request_id,
                "error": "User not found"
            })
        }
        Err(e) => {
            error!(
                event = "get_user_failed",
                request_id = %request_id,
                user_snowflake_id = user_snowflake_id,
                error = ?e,
                "User lookup failed"
            );
            
            json!({
                "success": false,
                "request_id": request_id,
                "error": format!("User lookup failed: {:?}", e)
            })
        }
    };

    send_response(redis_client, &response).await;
}

async fn handle_get_users(parsed_message: &Value, user_service: &UserService, redis_client: &Client) {
    // Check if we have user_ids (internal UUIDs) or user_snowflake_ids (Discord snowflakes)
    if let Some(user_ids_array) = parsed_message.get("user_ids").and_then(|v| v.as_array()) {
        // Handle internal UUID lookup
        handle_get_users_by_internal_ids(parsed_message, user_service, redis_client, user_ids_array).await;
        return;
    }

    let user_snowflake_ids: Vec<i64> = match parsed_message.get("user_snowflake_ids").and_then(|v| v.as_array()) {
        Some(arr) => {
            let mut ids = Vec::new();
            for item in arr {
                if let Some(id_str) = item.as_str() {
                    match id_str.parse::<i64>() {
                        Ok(id) => ids.push(id),
                        Err(_) => {
                            error!("Invalid user_snowflake_id in array: {}", id_str);
                            return;
                        }
                    }
                } else {
                    error!("Non-string value in user_snowflake_ids array");
                    return;
                }
            }
            ids
        }
        None => {
            error!("Missing 'user_snowflake_ids' or 'user_ids' field for get_users action");
            return;
        }
    };

    let request_id = parsed_message.get("request_id").and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let response = match user_service.get_users(user_snowflake_ids.clone()).await {
        Ok(users_map) => {
            info!(
                event = "get_users_success",
                request_id = %request_id,
                requested_count = user_snowflake_ids.len(),
                found_count = users_map.len(),
                "Bulk user lookup successful"
            );
            
            let users_data: Vec<Value> = users_map.into_iter().map(|(snowflake_id, user_info)| {
                json!({
                    "user_id": user_info.id,
                    "snowflake_id": snowflake_id.to_string(),
                    "username": user_info.username,
                    "global_name": user_info.global_name,
                    "avatar": user_info.avatar,
                    "banner": user_info.banner,
                    "guild_role": user_info.guild_role,
                    "superadmin": user_info.superadmin
                })
            }).collect();
            
            json!({
                "success": true,
                "request_id": request_id,
                "data": {
                    "users": users_data,
                    "requested_count": user_snowflake_ids.len(),
                    "found_count": users_data.len()
                }
            })
        }
        Err(e) => {
            error!(
                event = "get_users_failed",
                request_id = %request_id,
                requested_count = user_snowflake_ids.len(),
                error = ?e,
                "Bulk user lookup failed"
            );
            
            json!({
                "success": false,
                "request_id": request_id,
                "error": format!("Bulk user lookup failed: {:?}", e)
            })
        }
    };

    send_response(redis_client, &response).await;
}

async fn handle_get_users_by_internal_ids(parsed_message: &Value, user_service: &UserService, redis_client: &Client, user_ids_array: &Vec<Value>) {
    let user_internal_ids: Vec<String> = {
        let mut ids = Vec::new();
        for item in user_ids_array {
            if let Some(id_str) = item.as_str() {
                ids.push(id_str.to_string());
            } else {
                error!("Non-string value in user_ids array");
                return;
            }
        }
        ids
    };

    let request_id = parsed_message.get("request_id").and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let response = match user_service.get_users_by_internal_ids(user_internal_ids.clone()).await {
        Ok(users_map) => {
            info!(
                event = "get_users_by_internal_ids_success",
                request_id = %request_id,
                requested_count = user_internal_ids.len(),
                found_count = users_map.len(),
                "Bulk user lookup by internal IDs successful"
            );
            
            let users_data: Vec<Value> = users_map.into_iter().map(|(internal_id, user_info)| {
                json!({
                    "user_id": internal_id,
                    "snowflake_id": user_info.snowflake_id.to_string(),
                    "username": user_info.username,
                    "global_name": user_info.global_name,
                    "avatar": user_info.avatar,
                    "banner": user_info.banner,
                    "guild_role": user_info.guild_role,
                    "superadmin": user_info.superadmin
                })
            }).collect();
            
            json!({
                "success": true,
                "request_id": request_id,
                "data": {
                    "users": users_data,
                    "requested_count": user_internal_ids.len(),
                    "found_count": users_data.len()
                }
            })
        }
        Err(e) => {
            error!(
                event = "get_users_by_internal_ids_failed",
                request_id = %request_id,
                requested_count = user_internal_ids.len(),
                error = ?e,
                "Bulk user lookup by internal IDs failed"
            );
            
            json!({
                "success": false,
                "request_id": request_id,
                "error": format!("Bulk user lookup by internal IDs failed: {:?}", e)
            })
        }
    };

    send_response(redis_client, &response).await;
}

async fn send_response(redis_client: &Client, response: &Value) {
    match redis_client.get_multiplexed_async_connection().await {
        Ok(mut conn) => {
            let response_str = response.to_string();
            match conn.lpush::<&str, String, i32>("chloe-responses", response_str).await {
                Ok(_) => {
                    info!(
                        event = "response_sent",
                        request_id = response.get("request_id").and_then(|v| v.as_str()).unwrap_or("unknown"),
                        "Response sent to chloe-responses queue"
                    );
                }
                Err(e) => {
                    error!(
                        event = "response_send_failed",
                        error = ?e,
                        "Failed to send response to Redis"
                    );
                }
            }
        }
        Err(e) => {
            error!(
                event = "redis_connection_failed",
                error = ?e,
                "Failed to get Redis connection for response"
            );
        }
    }
}

async fn handle_get_user_auth(parsed_message: &Value, user_service: &UserService, redis_client: &Client) {
    let user_snowflake_str = match parsed_message.get("snowflake_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            error!("Missing 'snowflake_id' field for get_user_auth action");
            return;
        }
    };

    let user_snowflake_id: i64 = match user_snowflake_str.parse() {
        Ok(id) => id,
        Err(_) => {
            error!("Invalid snowflake_id format: {}", user_snowflake_str);
            return;
        }
    };

    let request_id = parsed_message.get("request_id").and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let response = match user_service.get_user_auth_info(user_snowflake_id).await {
        Ok(Some(auth_info)) => {
            info!(
                event = "get_user_auth_success",
                request_id = %request_id,
                user_snowflake_id = user_snowflake_id,
                user_internal_id = %auth_info.user.id,
                guild_count = auth_info.guilds.len(),
                superadmin = auth_info.user.superadmin,
                "User auth info lookup successful"
            );
            
            let guilds_data: Vec<Value> = auth_info.guilds.iter().map(|guild| {
                json!({
                    "guild_id": guild.guild_id,
                    "guild_snowflake_id": guild.guild_snowflake_id.to_string(),
                    "guild_name": guild.guild_name,
                    "role": guild.role
                })
            }).collect();
            
            json!({
                "success": true,
                "request_id": request_id,
                "data": {
                    "user": {
                        "user_id": auth_info.user.id,
                        "snowflake_id": auth_info.user.snowflake_id.to_string(),
                        "username": auth_info.user.username,
                        "global_name": auth_info.user.global_name,
                        "avatar": auth_info.user.avatar,
                        "banner": auth_info.user.banner,
                        "superadmin": auth_info.user.superadmin
                    },
                    "guilds": guilds_data,
                    "guild_count": auth_info.guilds.len()
                }
            })
        }
        Ok(None) => {
            info!(
                event = "get_user_auth_not_found",
                request_id = %request_id,
                user_snowflake_id = user_snowflake_id,
                "User not found for auth info"
            );
            
            json!({
                "success": false,
                "request_id": request_id,
                "error": "User not found"
            })
        }
        Err(e) => {
            error!(
                event = "get_user_auth_failed",
                request_id = %request_id,
                user_snowflake_id = user_snowflake_id,
                error = ?e,
                "User auth info lookup failed"
            );
            
            json!({
                "success": false,
                "request_id": request_id,
                "error": format!("User auth info lookup failed: {:?}", e)
            })
        }
    };

    send_response(redis_client, &response).await;
}