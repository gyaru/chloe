use crate::settings::Settings;
use anyhow::{Context, Result};
use lazy_static::lazy_static;
use regex::Regex;
use reqwest::Client;
use serde_json::{Value, json};
use std::{collections::VecDeque, env, sync::Arc};
use tokio::sync::RwLock;
use tracing::{error, info};

lazy_static! {
    static ref USER_MENTION_RE: Regex = Regex::new(r"<@!?(\d+)>").unwrap();
    static ref CHANNEL_MENTION_RE: Regex = Regex::new(r"<#(\d+)>").unwrap();
    static ref ROLE_MENTION_RE: Regex = Regex::new(r"<@&(\d+)>").unwrap();
    static ref EMOJI_RE: Regex = Regex::new(r"<a?:\w+:\d+>").unwrap();
    static ref WHITESPACE_RE: Regex = Regex::new(r"\s+").unwrap();
    static ref URL_RE: Regex = Regex::new(r"https?://[^\s]+").unwrap();
}

#[derive(Clone, Debug)]
pub struct MessageContext {
    pub user_display_name: String,
    pub user_id: u64,
    pub content: String,
    pub is_bot: bool,
    pub channel_id: u64,
}

#[derive(Clone, Debug)]
pub struct ConversationContext {
    pub current_user: String,
    pub current_message: String,
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

        let optimized_user_prompt = self.optimize_user_prompt(&context.current_message);
        let enriched_system_prompt =
            self.enrich_system_prompt_with_context(&global_settings.prompt, &context);

        let original_tokens = self.estimate_tokens(&global_settings.prompt)
            + self.estimate_tokens(&context.current_message);
        let optimized_tokens = self.estimate_tokens(&enriched_system_prompt)
            + self.estimate_tokens(&optimized_user_prompt);

        info!(
            event = "context_aware_prompting",
            original_tokens = original_tokens,
            final_tokens = optimized_tokens,
            context_effect = if optimized_tokens > original_tokens { "added" } else { "optimized" },
            recent_messages_count = context.recent_messages.len(),
            current_user = %context.current_user,
            "Processing message with conversation context"
        );

        self.prompt_gemini(&enriched_system_prompt, &optimized_user_prompt)
            .await
    }

    fn enrich_system_prompt_with_context(
        &self,
        base_prompt: &str,
        context: &ConversationContext,
    ) -> String {
        let mut enriched = self.optimize_system_prompt(base_prompt);
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

        enriched.push_str("\n## Current Context\n");
        enriched.push_str(&format!("Current user: {}\n", context.current_user));
        
        if let Some(ref referenced_msg) = context.referenced_message {
            enriched.push_str("\n## Message Being Replied To\n");
            enriched.push_str(&format!(
                "{}: {}\n",
                referenced_msg.user_display_name,
                self.optimize_user_prompt_preserve_mentions(&referenced_msg.content)
            ));
            enriched.push_str("\n## Current Reply\n");
        }
        
        enriched.push_str(&format!(
            "Current message: {}\n",
            self.optimize_user_prompt(&context.current_message)
        ));

        if !context.recent_messages.is_empty() {
            enriched.push_str("\n## Recent Conversation:\n");
            for msg in context.recent_messages.iter().take(8) {
                if msg.is_bot {
                    enriched.push_str(&format!("Chloe: {}\n", msg.content));
                } else {
                    enriched.push_str(&format!(
                        "{}: {}\n",
                        msg.user_display_name,
                        self.optimize_user_prompt_preserve_mentions(&msg.content)
                    ));
                }
            }
        }

        enriched
    }

    fn optimize_user_prompt_preserve_mentions(&self, prompt: &str) -> String {
        let mut optimized = prompt.to_string();

        optimized = EMOJI_RE.replace_all(&optimized, ":emoji:").to_string();
        optimized = URL_RE.replace_all(&optimized, "[link]").to_string();
        optimized = WHITESPACE_RE.replace_all(&optimized, " ").to_string();

        if optimized.len() > 400 {
            optimized = format!("{}...[truncated]", &optimized[..400]);
        }

        optimized.trim().to_string()
    }

    fn estimate_tokens(&self, text: &str) -> usize {
        (text.len() as f32 / 4.0).ceil() as usize
    }

    fn optimize_system_prompt(&self, system_prompt: &str) -> String {
        let mut optimized = system_prompt.to_string();

        if optimized.len() > 2000 {
            optimized = optimized
                .lines()
                .map(|line| {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with("####") {
                        ""
                    } else if trimmed.starts_with("###") {
                        trimmed.trim_start_matches('#').trim()
                    } else if trimmed.starts_with("##") {
                        trimmed.trim_start_matches('#').trim()
                    } else if trimmed.starts_with('#') {
                        trimmed.trim_start_matches('#').trim()
                    } else {
                        trimmed
                    }
                })
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" ");

            optimized = Regex::new(r"\[([^\]]+)\]\([^)]+\)")
                .unwrap()
                .replace_all(&optimized, "$1")
                .to_string();

            if optimized.len() > 1500 {
                if let Some(truncate_pos) = optimized.char_indices().nth(1500) {
                    optimized = format!(
                        "{}...[system prompt truncated for efficiency]",
                        &optimized[..truncate_pos.0]
                    );
                }
            }
        }

        WHITESPACE_RE
            .replace_all(&optimized, " ")
            .trim()
            .to_string()
    }

    fn optimize_user_prompt(&self, prompt: &str) -> String {
        let mut optimized = prompt.to_string();
        optimized = EMOJI_RE.replace_all(&optimized, ":emoji:").to_string();
        optimized = URL_RE.replace_all(&optimized, "[link]").to_string();
        optimized = WHITESPACE_RE.replace_all(&optimized, " ").to_string();

        if optimized.len() > 800 {
            optimized = format!("{}...[message truncated]", &optimized[..800]);
        }

        optimized.trim().to_string()
    }

    async fn send_request(&self, url: &str, combined_prompt: &str) -> Result<String> {
        let request_body = json!({
            "contents": [
                {
                    "parts": [
                        {
                            "text": combined_prompt
                        }
                    ]
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
