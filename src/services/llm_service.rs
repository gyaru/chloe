use crate::settings::Settings;
use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::{Value, json};
use std::{collections::VecDeque, env, sync::Arc};
use tokio::sync::RwLock;
use tracing::{error, info};


#[derive(Clone, Debug)]
pub struct MessageContext {
    pub user_display_name: String,
    pub user_id: u64,
    pub content: String,
    pub is_bot: bool,
    pub channel_id: u64,
    pub images: Vec<ImageData>,
}

#[derive(Clone, Debug)]
pub struct ImageData {
    pub base64_data: String,
    pub mime_type: String,
}

#[derive(Clone, Debug)]
pub struct ConversationContext {
    pub current_user: String,
    pub current_message: String,
    pub current_images: Vec<ImageData>,
    pub recent_messages: Vec<MessageContext>,
    pub user_info: Vec<UserInfo>,
    pub referenced_message: Option<MessageContext>,
}

#[derive(Clone, Debug)]
pub struct UserInfo {
    pub display_name: String,
    pub user_id: u64,
    pub is_bot: bool,
}

pub struct LlmService {
    client: Client,
    api_key: String,
    settings: Arc<Settings>,
    conversation_history: Arc<RwLock<std::collections::HashMap<u64, VecDeque<MessageContext>>>>,
}

impl LlmService {
    pub fn new(settings: Arc<Settings>) -> Result<Self> {
        let api_key =
            env::var("GEMINI_API_KEY").context("GEMINI_API_KEY environment variable not set")?;

        if api_key.is_empty() {
            return Err(anyhow::anyhow!("GEMINI_API_KEY cannot be empty"));
        }

        let client = Client::new();

        info!(
            event = "llm_service_initialized",
            "LLM service initialized successfully"
        );

        Ok(Self {
            client,
            api_key,
            settings,
            conversation_history: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    pub async fn prompt_gemini(&self, system_prompt: &str, prompt: &str) -> Result<String> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-preview-05-20:generateContent?key={}",
            self.api_key
        );

        let combined_prompt = if system_prompt.is_empty() {
            prompt.to_string()
        } else {
            format!("{}\n\n{}", system_prompt, prompt)
        };

        self.send_request(&url, &combined_prompt).await
    }


    pub async fn prompt_with_context(&self, context: ConversationContext) -> Result<String> {
        let global_settings = self.settings.get_global_settings().await;

        let enriched_system_prompt =
            self.enrich_system_prompt_with_context(&global_settings.prompt, &context);

        info!(
            event = "context_aware_prompting",
            recent_messages_count = context.recent_messages.len(),
            current_user = %context.current_user,
            images_count = context.current_images.len(),
            "Processing message with conversation context"
        );

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-preview-05-20:generateContent?key={}",
            self.api_key
        );

        let combined_prompt = if enriched_system_prompt.is_empty() {
            context.current_message.clone()
        } else {
            enriched_system_prompt
        };

        self.send_request_with_images(&url, &combined_prompt, &context.current_images)
            .await
    }

    fn enrich_system_prompt_with_context(
        &self,
        base_prompt: &str,
        context: &ConversationContext,
    ) -> String {
        let mut enriched = base_prompt.to_string();
        if !context.user_info.is_empty() {
            enriched.push_str("\n\n## User Information\n");
            enriched.push_str(
                "When you see Discord mentions like <@123456>, here's who they refer to:\n",
            );
            for user in &context.user_info {
                if user.is_bot {
                    enriched.push_str(&format!(
                        "- <@{}> = {} (Bot)\n",
                        user.user_id, user.display_name
                    ));
                } else {
                    enriched.push_str(&format!(
                        "- <@{}> = {} (User)\n",
                        user.user_id, user.display_name
                    ));
                }
            }
        }

        // add conversation context if available
        if !context.recent_messages.is_empty() {
            enriched.push_str("\n## Recent Conversation:\n");
            for msg in context.recent_messages.iter() {
                if msg.is_bot {
                    enriched.push_str(&format!("Chloe: {}\n", msg.content));
                } else {
                    enriched.push_str(&format!(
                        "{}: {}\n",
                        msg.user_display_name,
                        msg.content
                    ));
                }
            }
            // Note: Don't add referenced_message here since it's already the first item in recent_messages
        } else if let Some(ref referenced_msg) = context.referenced_message {
            // No chain, just show the single referenced message
            enriched.push_str("\n## Previous Message:\n");
            enriched.push_str(&format!(
                "{}: {}\n",
                referenced_msg.user_display_name,
                referenced_msg.content
            ));
        }
        
        enriched.push_str(&format!(
            "\n## Current Message to Respond To:\n{}: {}",
            context.current_user,
            context.current_message
        ));

        enriched
    }


    fn estimate_tokens(&self, text: &str) -> usize {
        (text.len() as f32 / 4.0).ceil() as usize
    }



    async fn send_request(&self, url: &str, combined_prompt: &str) -> Result<String> {
        self.send_request_with_images(url, combined_prompt, &[]).await
    }

    async fn send_request_with_images(&self, url: &str, combined_prompt: &str, images: &[ImageData]) -> Result<String> {
        let mut parts = vec![json!({
            "text": combined_prompt
        })];

        // Add images to the request
        for image in images {
            parts.push(json!({
                "inline_data": {
                    "mime_type": image.mime_type,
                    "data": image.base64_data
                }
            }));
        }

        let request_body = json!({
            "contents": [
                {
                    "parts": parts
                }
            ]
        });

        info!(
            event = "gemini_api_request",
            url = url.split('?').next().unwrap_or(url),
            prompt_chars = combined_prompt.len(),
            estimated_tokens = self.estimate_tokens(combined_prompt),
            prompt = %self.format_prompt_for_display(combined_prompt),
            "Sending request to Gemini API"
        );

        let response = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send request to Gemini API")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                event = "gemini_api_error",
                status_code = %status,
                error_text = %error_text,
                "Gemini API request failed"
            );
            return Err(anyhow::anyhow!(
                "API request failed with status {}: {}",
                status,
                error_text
            ));
        }

        let response_json: Value = response
            .json()
            .await
            .context("Failed to parse JSON response from Gemini API")?;

        let content = response_json
            .get("candidates")
            .and_then(|candidates| candidates.get(0))
            .and_then(|candidate| candidate.get("content"))
            .and_then(|content| content.get("parts"))
            .and_then(|parts| parts.get(0))
            .and_then(|part| part.get("text"))
            .and_then(|text| text.as_str())
            .context("Failed to extract text from Gemini API response")?;

        info!(
            event = "gemini_api_response",
            response_chars = content.len(),
            estimated_tokens = self.estimate_tokens(content),
            response = %self.format_response_for_display(content),
            "Received response from Gemini API"
        );

        Ok(content.to_string())
    }

    fn format_prompt_for_display(&self, prompt: &str) -> String {
        let lines: Vec<&str> = prompt.lines().collect();
        let mut formatted = String::new();

        for (i, line) in lines.iter().enumerate() {
            if lines.len() > 10 {
                formatted.push_str(&format!("{:3} | {}\n", i + 1, line));
            } else {
                formatted.push_str(&format!("{}\n", line));
            }

            if i > 50 {
                formatted.push_str("... [truncated for display]\n");
                break;
            }
        }

        formatted.trim_end().to_string()
    }

    fn format_response_for_display(&self, response: &str) -> String {
        // Format the response nicely for logging
        if response.len() <= 500 {
            response.to_string()
        } else {
            format!(
                "{}...\n[... {} chars omitted ...]\n{}",
                &response[..200],
                response.len() - 400,
                &response[response.len() - 200..]
            )
        }
    }
}
