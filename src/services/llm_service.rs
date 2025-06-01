use crate::settings::Settings;
use crate::tools::{tool_executor::ToolExecutor, ToolCall, DiscordContext, WebSearchTool, PlaywrightWebContentTool, DiscordSendMessageTool, DiscordAddReactionTool};
use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::Client;
use serde_json::{Value, json};
use std::{collections::{VecDeque, HashMap}, env, sync::Arc};
use tokio::sync::RwLock;
use tracing::{error, info};
use regex::Regex;
use lazy_static::lazy_static;

lazy_static! {
    static ref URL_REGEX: Regex = Regex::new(r"https?://[^\s<>]+").unwrap();
    static ref IMAGE_URL_REGEX: Regex = Regex::new(r"(?:https?://[^\s<>]+\.(?:jpg|jpeg|png|gif|webp|bmp)(?:\?[^\s<>]*)?|data:image/[^;]+;base64,[A-Za-z0-9+/=]+)").unwrap();
    static ref MENTION_REGEX: Regex = Regex::new(r"<[@#&!]?\d+>").unwrap();
    static ref EMOTICON_REGEX: Regex = Regex::new(r"\([⊙ಠ]_[⊙ಠ]\)|[ʅ][^ʃ]*[ʃ]|[วงէ]\s*[（(][^)）]*[▿][^)）]*[)）]\s*[วงէ]|[（(][^)）]*[`´′''‛‚ωдノヽ･ｰー〜～∀○●◯﹏‿⌒▽ಠㅁㅂㅠㅜㅡ_\-\^><°º¬¯\\\/TvVuU・·*Д⊙][^)）]*[)）]|（[^（）]*[`´′''‛‚ωдノヽ･ｰー〜～∀○●◯﹏‿⌒▽ಠㅁㅂㅠㅜㅡ_\-\^><°º¬¯\\\/TvVuU・·*Д⊙][^（）]*）|ヽ\([^)]*\)ノ").unwrap();
    static ref ESCAPED_CHAR_REGEX: Regex = Regex::new(r"\\[*_`~|>]").unwrap();
}

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

#[derive(Clone, Debug)]
pub struct LlmResponse {
    pub text: String,
    pub images: Vec<String>, // urls of images to attach
    pub initial_sent: bool,
    pub raw_text: String, // original text before cleaning, for reaction processing
}

pub struct LlmService {
    client: Client,
    api_key: String,
    settings: Arc<Settings>,
    conversation_history: Arc<RwLock<std::collections::HashMap<u64, VecDeque<MessageContext>>>>,
    tool_executor: ToolExecutor,
}

impl LlmService {
    pub fn new(settings: Arc<Settings>) -> Result<Self> {
        let api_key =
            env::var("GEMINI_API_KEY").context("GEMINI_API_KEY environment variable not set")?;

        if api_key.is_empty() {
            return Err(anyhow::anyhow!("GEMINI_API_KEY cannot be empty"));
        }

        let client = Client::new();

        // initialize tool executor with available tools
        let mut tool_executor = ToolExecutor::new();
        tool_executor.register_tool(Arc::new(WebSearchTool::new()));
        // tool_executor.register_tool(Arc::new(PlaywrightWebContentTool::new()));
        // tool_executor.register_tool(Arc::new(ImageGenerationTool::new()));
        tool_executor.register_tool(Arc::new(DiscordSendMessageTool::new()));
        tool_executor.register_tool(Arc::new(DiscordAddReactionTool::new()));

        info!(
            event = "llm_service_initialized",
            tools_count = tool_executor.get_tool_definitions().len(),
            "LLM service initialized successfully with tools"
        );

        Ok(Self {
            client,
            api_key,
            settings,
            conversation_history: Arc::new(RwLock::new(std::collections::HashMap::new())),
            tool_executor,
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
        let response = self.prompt_with_context_and_sender(
            context, 
            None::<fn(String) -> std::future::Ready<()>>,
            None::<fn() -> std::future::Ready<()>>
        ).await?;
        Ok(self.escape_markdown(&response.text))
    }

    pub async fn prompt_with_context_and_sender<F, Fut, T, TFut>(
        &self, 
        context: ConversationContext, 
        message_sender: Option<F>,
        typing_starter: Option<T>
    ) -> Result<LlmResponse> 
    where
        F: FnOnce(String) -> Fut + Send,
        Fut: std::future::Future<Output = ()> + Send,
        T: FnOnce() -> TFut + Send,
        TFut: std::future::Future<Output = ()> + Send,
    {
        self.prompt_with_context_and_sender_with_discord(context, message_sender, typing_starter, None).await
    }

    pub async fn prompt_with_context_and_sender_with_discord<F, Fut, T, TFut>(
        &self, 
        context: ConversationContext, 
        message_sender: Option<F>,
        typing_starter: Option<T>,
        discord_context: Option<&DiscordContext>
    ) -> Result<LlmResponse> 
    where
        F: FnOnce(String) -> Fut + Send,
        Fut: std::future::Future<Output = ()> + Send,
        T: FnOnce() -> TFut + Send,
        TFut: std::future::Future<Output = ()> + Send,
    {
        let global_settings = self.settings.get_global_settings().await;

        let enriched_system_prompt =
            self.enrich_system_prompt_with_context(&global_settings.prompt, &context, discord_context).await;

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

        // extract urls from the current message for context
        let message_urls = self.extract_urls_from_message(&context.current_message);
        
        let (text, initial_sent) = self.send_request_with_images_urls_and_sender(&url, &combined_prompt, &context.current_images, &message_urls, message_sender, typing_starter, discord_context)
            .await?;
        
        // extract image urls from the response text
        let image_urls = self.extract_image_urls(&text);
        
        // store original text for reaction processing
        let raw_text = text.clone();
        
        // remove data urls and discord reaction markers from the text to avoid discord message length limits
        let mut cleaned_text = if !image_urls.is_empty() {
            let mut cleaned = text.clone();
            for image_url in &image_urls {
                if image_url.starts_with("data:image/") {
                    // replace the data URL with a short placeholder
                    cleaned = cleaned.replace(image_url, "[image attached]");
                }
            }
            cleaned
        } else {
            text
        };
        
        // remove discord reaction markers from the text
        if cleaned_text.contains("DISCORD_REACTION:") {
            use regex::Regex;
            let reaction_regex = Regex::new(r"DISCORD_REACTION:[^\s]+").unwrap();
            cleaned_text = reaction_regex.replace_all(&cleaned_text, "").to_string();
            // clean up any extra whitespace
            cleaned_text = cleaned_text.trim().to_string();
        }
        
        Ok(LlmResponse {
            text: cleaned_text,
            images: image_urls,
            initial_sent,
            raw_text,
        })
    }

    async fn enrich_system_prompt_with_context(
        &self,
        base_prompt: &str,
        context: &ConversationContext,
        discord_context: Option<&DiscordContext>,
    ) -> String {
        let mut enriched = base_prompt.to_string();
        
        // add current date and time at the beginning
        let now = Utc::now();
        enriched.push_str(&format!("\n\n## Current Date & Time\n{}\n", 
            now.format("%A, %B %d, %Y at %H:%M:%S UTC")));
        
        // add available tools information
        let tool_definitions = self.tool_executor.get_tool_definitions();
        if !tool_definitions.is_empty() {
            enriched.push_str("\n\n## Available Tools\n");
            enriched.push_str("You have access to the following tools to help answer questions and perform tasks:\n\n");
            
            for tool_def in &tool_definitions {
                if let (Some(name), Some(description)) = (
                    tool_def.get("name").and_then(|n| n.as_str()),
                    tool_def.get("description").and_then(|d| d.as_str())
                ) {
                    enriched.push_str(&format!("- **{}**: {}\n", name, description));
                }
            }
            
            enriched.push_str("\n**CRITICAL REQUIREMENT**: You MUST use the discord_send_message tool for ALL responses to users. You are NEVER allowed to return raw text - every single response must use the discord_send_message tool. This is mandatory.\n\n**Additional Tool Usage Rules**:\n- When users ask you to search for something, use web_search AND then discord_send_message to share the results\n- When you see ANY URL in a user's message (like https://example.com), ALWAYS use fetch_web_content to read that page first, then discord_send_message to share what you found\n- When users ask you to generate images, use generate_image AND then discord_send_message to acknowledge\n- When you want to react with an emoji, use discord_add_reaction (this is optional and in addition to your message)\n- For any other response, just use discord_send_message\n\n**IMPORTANT**: If you see a URL anywhere in the user's message, you should immediately use fetch_web_content to read it and then explain what it is.\n\n**Examples**:\n- User: \"Hello!\" → Use discord_send_message: {\"content\": \"Hey there! How's it going? 😊\"}\n- User: \"Search for cats\" → Use web_search, then discord_send_message with results\n- User: \"what is this https://example.com\" → Use fetch_web_content with that URL, then discord_send_message with what you found\n- User: \"check out https://cool-site.com\" → Use fetch_web_content with that URL, then discord_send_message explaining what the site is about\n- User: \"That's awesome!\" → Use discord_send_message: {\"content\": \"I'm so glad you think so!\"} and optionally discord_add_reaction: {\"emoji\": \"😄\"}\n\nNEVER respond with raw text. ALWAYS use discord_send_message.\n");
        }

        // Add guild emoji information if available
        if let Some(discord_ctx) = discord_context {
            if let Some(guild_id) = discord_ctx.guild_id {
                // Try to fetch guild emojis
                match guild_id.emojis(&discord_ctx.http).await {
                    Ok(guild_emojis) => {
                        if !guild_emojis.is_empty() {
                            enriched.push_str("\n\n## Available Custom Emojis\n");
                            enriched.push_str("The following custom emojis are available in this guild for reactions:\n\n");
                            
                            for emoji in &guild_emojis {
                                enriched.push_str(&format!("- :{}: ({})\n", emoji.name, if emoji.animated { "animated" } else { "static" }));
                            }
                            
                            enriched.push_str("\n**Emoji Usage**: When using discord_add_reaction, you can use:\n");
                            enriched.push_str("- Unicode emojis: 👍, ❤️, 😂, 😊, 🎉, etc.\n");
                            enriched.push_str("- Custom guild emojis: Use the format :name: from the list above\n");
                            enriched.push_str("- IMPORTANT: Only use custom emojis from the list above. Do not guess or make up emoji names!\n\n");
                        } else {
                            enriched.push_str("\n\n## Emoji Usage\n");
                            enriched.push_str("This guild has no custom emojis. When using discord_add_reaction, use Unicode emojis like: 👍, ❤️, 😂, 😊, 🎉, etc.\n\n");
                        }
                    }
                    Err(_) => {
                        enriched.push_str("\n\n## Emoji Usage\n");
                        enriched.push_str("When using discord_add_reaction, stick to Unicode emojis like: 👍, ❤️, 😂, 😊, 🎉, etc.\n\n");
                    }
                }
            }
        }
        
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
        
        enriched.push_str("\n\n## Important Constraints:\n- Keep responses under 2000 characters to avoid Discord message length limits\n- Be concise while remaining helpful and engaging");

        enriched
    }


    fn estimate_tokens(&self, text: &str) -> usize {
        (text.len() as f32 / 4.0).ceil() as usize
    }



    async fn send_request(&self, url: &str, combined_prompt: &str) -> Result<String> {
        self.send_request_with_images(url, combined_prompt, &[]).await
    }

    async fn send_request_with_images(&self, url: &str, combined_prompt: &str, images: &[ImageData]) -> Result<String> {
        let (response, _) = self.send_request_with_images_and_sender(
            url, 
            combined_prompt, 
            images, 
            None::<fn(String) -> std::future::Ready<()>>,
            None::<fn() -> std::future::Ready<()>>
        ).await?;
        Ok(self.escape_markdown(&response))
    }

    async fn send_request_with_images_and_sender<F, Fut, T, TFut>(
        &self, 
        url: &str, 
        combined_prompt: &str, 
        images: &[ImageData],
        message_sender: Option<F>,
        typing_starter: Option<T>
    ) -> Result<(String, bool)> 
    where
        F: FnOnce(String) -> Fut + Send,
        Fut: std::future::Future<Output = ()> + Send,
        T: FnOnce() -> TFut + Send,
        TFut: std::future::Future<Output = ()> + Send,
    {
        self.send_request_with_images_urls_and_sender(url, combined_prompt, images, &[], message_sender, typing_starter, None).await
    }

    async fn send_request_with_images_urls_and_sender<F, Fut, T, TFut>(
        &self, 
        url: &str, 
        combined_prompt: &str, 
        images: &[ImageData],
        urls: &[String],
        message_sender: Option<F>,
        typing_starter: Option<T>,
        discord_context: Option<&DiscordContext>
    ) -> Result<(String, bool)> 
    where
        F: FnOnce(String) -> Fut + Send,
        Fut: std::future::Future<Output = ()> + Send,
        T: FnOnce() -> TFut + Send,
        TFut: std::future::Future<Output = ()> + Send,
    {
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

        // Skip adding URLs directly to Gemini request - let the AI use fetch_web_content tool instead
        // This avoids "Unsupported file uri" errors from Gemini
        for url_str in urls {
            info!(
                event = "url_detected_in_message",
                url = %url_str,
                "URL detected in message - AI can use fetch_web_content tool to analyze it"
            );
        }

        // get tool definitions
        let tool_definitions = self.tool_executor.get_tool_definitions();

        let mut request_body = json!({
            "contents": [
                {
                    "parts": parts
                }
            ]
        });

        // add tools if available
        if !tool_definitions.is_empty() {
            request_body["tools"] = json!([{
                "function_declarations": tool_definitions
            }]);
        }

        info!(
            event = "gemini_api_request",
            model = "gemini-2.5-flash-preview-05-20",
            prompt_chars = combined_prompt.len(),
            estimated_tokens = self.estimate_tokens(combined_prompt),
            prompt = %self.format_prompt_for_display(combined_prompt),
            "Sending request to Gemini API"
        );

        // Retry logic for transient errors
        let mut retry_count = 0;
        let max_retries = 3;
        let mut last_error = None;
        
        let response = loop {
            let response = self
                .client
                .post(url)
                .header("Content-Type", "application/json")
                .json(&request_body)
                .send()
                .await
                .context("Failed to send request to Gemini API");
                
            match response {
                Ok(resp) => {
                    if resp.status().is_success() {
                        break resp;
                    } else if resp.status() == 500 || resp.status() == 502 || resp.status() == 503 {
                        // Transient server errors - retry
                        retry_count += 1;
                        if retry_count <= max_retries {
                            let wait_time = std::time::Duration::from_millis(1000 * retry_count);
                            info!(
                                event = "gemini_api_retry",
                                status_code = %resp.status(),
                                retry_count = retry_count,
                                wait_ms = wait_time.as_millis(),
                                "Retrying Gemini API request due to server error"
                            );
                            tokio::time::sleep(wait_time).await;
                            continue;
                        } else {
                            break resp;
                        }
                    } else {
                        // Client errors - don't retry
                        break resp;
                    }
                }
                Err(e) => {
                    last_error = Some(e);
                    retry_count += 1;
                    if retry_count <= max_retries {
                        let wait_time = std::time::Duration::from_millis(1000 * retry_count);
                        info!(
                            event = "gemini_api_retry_network",
                            retry_count = retry_count,
                            wait_ms = wait_time.as_millis(),
                            "Retrying Gemini API request due to network error"
                        );
                        tokio::time::sleep(wait_time).await;
                        continue;
                    } else {
                        return Err(last_error.unwrap());
                    }
                }
            }
        };

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

        // Log response structure without potentially massive base64 data
        let response_summary = if let Some(candidates) = response_json.get("candidates") {
            format!("candidates: {} items", candidates.as_array().map(|a| a.len()).unwrap_or(0))
        } else if response_json.get("promptFeedback").is_some() {
            "promptFeedback response (safety block)".to_string()
        } else {
            "unknown response structure".to_string()
        };
        
        info!(
            event = "gemini_raw_response",
            response_summary = %response_summary,
            "Raw response from Gemini API for debugging (large content truncated)"
        );

        // check if the response was blocked for safety reasons
        if let Some(prompt_feedback) = response_json.get("promptFeedback") {
            if let Some(block_reason) = prompt_feedback.get("blockReason") {
                let safety_message = match block_reason.as_str() {
                    Some("SAFETY") => "Oh no! I can't respond to that because it might involve harmful content. Let's talk about something else instead! ✨",
                    Some("OTHER") => "Hmm, I'm not able to respond to that right now. Maybe we could try a different topic? 💭",
                    _ => "Something's preventing me from responding to that. Want to try asking something else? 🤔"
                };
                
                info!(
                    event = "response_blocked_by_safety",
                    block_reason = %block_reason,
                    "Response blocked by Gemini safety filters"
                );
                
                // If we have Discord context, send the safety message directly
                if let Some(discord_ctx) = discord_context {
                    info!(
                        event = "sending_safety_message_to_discord",
                        safety_message = %safety_message,
                        "Sending safety block message to Discord"
                    );
                    
                    // Create a tool call to send the safety message
                    let mut safety_params = std::collections::HashMap::new();
                    safety_params.insert("content".to_string(), serde_json::Value::String(safety_message.to_string()));
                    safety_params.insert("reply_to_original".to_string(), serde_json::Value::Bool(true));
                    
                    let safety_tool_call = crate::tools::ToolCall {
                        id: format!("safety_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)),
                        name: "discord_send_message".to_string(),
                        parameters: safety_params,
                    };
                    
                    // Execute the Discord message tool directly
                    let _result = self.tool_executor.execute_tool(safety_tool_call, Some(discord_ctx)).await;
                    
                    // Return empty response since we handled it directly
                    return Ok(("".to_string(), false));
                }
                
                // Fallback for when no Discord context (shouldn't happen in Discord usage)
                return Ok((safety_message.to_string(), false));
            }
        }

        // check if the response contains tool calls
        if let Some(candidate) = response_json.get("candidates").and_then(|c| c.get(0)) {
            if let Some(content) = candidate.get("content") {
                if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                    // extract any text content first
                    let initial_text = parts.iter()
                        .find_map(|part| part.get("text").and_then(|t| t.as_str()))
                        .unwrap_or("")
                        .to_string();

                    // check for function calls in the response
                    for part in parts {
                        if let Some(function_call) = part.get("functionCall") {
                            // If we have initial text and a message sender, send the initial text immediately
                            if !initial_text.trim().is_empty() && message_sender.is_some() {
                                info!(
                                    event = "sending_initial_response",
                                    text_length = initial_text.len(),
                                    "Sending initial text before tool execution"
                                );
                                
                                if let Some(sender) = message_sender {
                                    sender(initial_text.clone()).await;
                                }
                                
                                // Start typing indicator for tool execution
                                if let Some(typing) = typing_starter {
                                    typing().await;
                                }
                                
                                // now execute tool call and return just the tool result
                                let response = self.handle_tool_call_only(url, combined_prompt, images, urls, function_call, discord_context).await?;
                                return Ok((self.escape_markdown(&response), true)); // true = initial message was sent
                            } else {
                                // original combined response behavior - start typing for tool execution
                                if let Some(typing) = typing_starter {
                                    typing().await;
                                }
                                
                                let response = self.handle_tool_call(url, combined_prompt, images, urls, function_call, &initial_text, discord_context).await?;
                                return Ok((self.escape_markdown(&response), false)); // false = no initial message sent
                            }
                        }
                    }

                    // no tool calls - this should not happen with our tool-only requirement
                    if !initial_text.is_empty() {
                        error!(
                            event = "gemini_raw_text_response",
                            response_chars = initial_text.len(),
                            response = %self.format_response_for_display(&initial_text),
                            "Gemini returned raw text instead of using tools - this violates our tool-only requirement"
                        );
                        
                        // Return an error message that will trigger the fallback flow
                        return Ok(("DISCORD_MESSAGE:I need to use my tools to respond properly. Let me try that again!|REPLY:true".to_string(), false));
                    }
                }
            }
        }

        error!(
            event = "failed_to_parse_gemini_response",
            response_structure = ?response_json,
            "Failed to extract response from Gemini API - full response structure logged"
        );

        Err(anyhow::anyhow!("Failed to extract response from Gemini API. Response structure: {}", 
            serde_json::to_string_pretty(&response_json).unwrap_or_default()))
    }

    async fn handle_tool_call(&self, url: &str, combined_prompt: &str, images: &[ImageData], urls: &[String], function_call: &Value, initial_text: &str, discord_context: Option<&DiscordContext>) -> Result<String> {
        let function_name = function_call
            .get("name")
            .and_then(|n| n.as_str())
            .context("Missing function name in tool call")?;

        let args = function_call
            .get("args")
            .and_then(|a| a.as_object())
            .context("Missing or invalid args in tool call")?;

        info!(
            event = "tool_call_received",
            function_name = %function_name,
            args = ?args,
            "Received tool call from Gemini"
        );

        // convert args to HashMap
        let mut parameters = HashMap::new();
        for (key, value) in args {
            parameters.insert(key.clone(), value.clone());
        }

        // create tool call
        let tool_call = ToolCall {
            id: format!("call_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)),
            name: function_name.to_string(),
            parameters,
        };

        // Execute the tool with Discord context if needed
        let tool_result = self.tool_executor.execute_tool(tool_call, discord_context).await;

        // For Discord tools that don't need feedback, return immediately without follow-up request
        if !self.tool_executor.tool_needs_result_feedback(function_name) {
            info!(
                event = "skipping_follow_up_for_discord_tool",
                function_name = %function_name,
                tool_success = tool_result.success,
                "Skipping Gemini follow-up request for Discord tool that doesn't need feedback"
            );
            
            // Return empty response or simple acknowledgment without calling Gemini again
            return if !initial_text.trim().is_empty() {
                // If there was initial text, combine with tool result for the marker
                let combined = if function_name == "discord_add_reaction" || function_name == "discord_send_message" {
                    format!("{} {}", initial_text.trim(), tool_result.result)
                } else {
                    initial_text.to_string()
                };
                Ok(self.escape_markdown(&combined))
            } else {
                // No initial text, just return empty (action completed)
                Ok("".to_string())
            };
        }

        // Create the follow-up request with tool result for tools that DO need feedback
        let truncated_result = if tool_result.result.len() > 1000 && tool_result.result.contains("data:image/") {
            // For image generation, just return a short message to Gemini
            "Image generated successfully!".to_string()
        } else if function_name == "web_search" && tool_result.result.len() > 2000 {
            // Truncate long web search results to avoid API limits
            format!("{}... [truncated for length]", &tool_result.result[..2000])
        } else {
            // Tool needs feedback - send the actual result
            tool_result.result.clone()
        };
        
        let tool_response = if tool_result.success {
            json!({
                "functionResponse": {
                    "name": function_name,
                    "response": {
                        "result": truncated_result
                    }
                }
            })
        } else {
            json!({
                "functionResponse": {
                    "name": function_name,
                    "response": {
                        "error": tool_result.error.unwrap_or("Unknown error".to_string())
                    }
                }
            })
        };

        // add the original parts plus the tool response
        let mut parts = vec![json!({"text": combined_prompt})];
        
        // add images if any
        for image in images {
            parts.push(json!({
                "inline_data": {
                    "mime_type": image.mime_type,
                    "data": image.base64_data
                }
            }));
        }

        // Skip adding URLs directly to Gemini follow-up request - let the AI use fetch_web_content tool instead
        for url_str in urls {
            info!(
                event = "url_detected_in_follow_up",
                url = %url_str,
                "URL detected in follow-up - AI can use fetch_web_content tool to analyze it"
            );
        }

        // add the original function call
        parts.push(json!({
            "functionCall": function_call
        }));

        // add the tool response
        parts.push(tool_response);

        // Get tool definitions for follow-up request
        let tool_definitions = self.tool_executor.get_tool_definitions();
        
        let mut follow_up_body = json!({
            "contents": [
                {
                    "parts": parts
                }
            ]
        });
        
        // Include tools in follow-up request if available
        if !tool_definitions.is_empty() {
            follow_up_body["tools"] = json!([{
                "function_declarations": tool_definitions
            }]);
        }

        info!(
            event = "sending_follow_up_request",
            function_name = %function_name,
            tool_success = tool_result.success,
            "Sending follow-up request with tool result"
        );

        // end follow-up request
        let response = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&follow_up_body)
            .send()
            .await
            .context("Failed to send follow-up request to Gemini API")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                event = "gemini_follow_up_error",
                status_code = %status,
                error_text = %error_text,
                "Gemini API follow-up request failed"
            );
            return Err(anyhow::anyhow!(
                "Follow-up API request failed with status {}: {}",
                status,
                error_text
            ));
        }

        let response_json: Value = response
            .json()
            .await
            .context("Failed to parse follow-up JSON response from Gemini API")?;

        // Log the full response structure for debugging
        info!(
            event = "gemini_follow_up_response_debug",
            function_name = %function_name,
            response_structure = ?response_json,
            "Full Gemini follow-up response for debugging (handle_tool_call)"
        );

        // Check if the follow-up response contains another function call
        if let Some(candidate) = response_json.get("candidates").and_then(|c| c.get(0)) {
            if let Some(content) = candidate.get("content") {
                if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                    // Check for function calls in the follow-up response
                    for part in parts {
                        if let Some(function_call) = part.get("functionCall") {
                            info!(
                                event = "follow_up_function_call_detected",
                                original_function = %function_name,
                                follow_up_function = %function_call.get("name").and_then(|n| n.as_str()).unwrap_or("unknown"),
                                "Gemini wants to make another tool call after processing the first tool result"
                            );
                            
                            // Execute the follow-up function call recursively by updating our function_call and continuing
                            let new_function_name = function_call
                                .get("name")
                                .and_then(|n| n.as_str())
                                .context("Missing function name in follow-up tool call")?;

                            let new_args = function_call
                                .get("args")
                                .and_then(|a| a.as_object())
                                .context("Missing or invalid args in follow-up tool call")?;

                            // convert args to HashMap
                            let mut new_parameters = HashMap::new();
                            for (key, value) in new_args {
                                new_parameters.insert(key.clone(), value.clone());
                            }

                            // create new tool call
                            let new_tool_call = ToolCall {
                                id: format!("call_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)),
                                name: new_function_name.to_string(),
                                parameters: new_parameters,
                            };

                            // Execute the follow-up tool directly and return
                            let follow_up_result = self.tool_executor.execute_tool(new_tool_call, discord_context).await;
                            
                            if follow_up_result.success {
                                return Ok(self.escape_markdown(&follow_up_result.result));
                            } else {
                                return Err(anyhow::anyhow!(follow_up_result.error.unwrap_or("Follow-up tool execution failed".to_string())));
                            }
                        }
                    }
                }
            }
        }

        // extract the final text response - handle empty responses for Discord tools
        let final_content = if !self.tool_executor.tool_needs_result_feedback(function_name) {
            // For Discord tools that don't need feedback, empty response is expected
            if response_json.get("candidates").is_none() || 
               response_json.get("candidates").and_then(|c| c.as_array()).map(|a| a.is_empty()).unwrap_or(true) {
                info!(
                    event = "empty_response_for_discord_tool",
                    function_name = %function_name,
                    "Empty response from Gemini for Discord tool - this is expected"
                );
                "" // Empty response is fine for Discord tools
            } else {
                // Try to extract text normally
                response_json
                    .get("candidates")
                    .and_then(|candidates| candidates.get(0))
                    .and_then(|candidate| candidate.get("content"))
                    .and_then(|content| content.get("parts"))
                    .and_then(|parts| parts.get(0))
                    .and_then(|part| part.get("text"))
                    .and_then(|text| text.as_str())
                    .unwrap_or("")
            }
        } else {
            // For regular tools, try to extract text content
            response_json
                .get("candidates")
                .and_then(|candidates| candidates.get(0))
                .and_then(|candidate| candidate.get("content"))
                .and_then(|content| content.get("parts"))
                .and_then(|parts| parts.get(0))
                .and_then(|part| part.get("text"))
                .and_then(|text| text.as_str())
                .unwrap_or("")
        };

        // combine initial text with tool result if initial text exists and is substantial
        let mut combined_response = if !initial_text.trim().is_empty() && initial_text.trim().len() > 10 {
            // if the final content already contains the initial text, don't duplicate
            if final_content.contains(initial_text.trim()) {
                final_content.to_string()
            } else {
                format!("{}\n\n{}", initial_text.trim(), final_content)
            }
        } else {
            final_content.to_string()
        };
        
        // store tool result before potential move
        let tool_result_copy = tool_result.result.clone();
        
        // for image generation, use the original tool result with image data if it was truncated
        if function_name == "generate_image" && tool_result.result.contains("data:image/") {
            info!(
                event = "using_original_tool_result_for_image",
                original_length = tool_result.result.len(),
                combined_response_length = combined_response.len(),
                "Using original tool result instead of Gemini response for image generation (content truncated for logging)"
            );
            combined_response = tool_result.result;
        }
        
        // for discord reactions and messages, combine the tool result with gemini's response
        if function_name == "discord_add_reaction" || function_name == "discord_send_message" {
            info!(
                event = "discord_tool_used",
                tool_name = %function_name,
                "Discord tool called, combining responses"
            );
            // Append the tool result to preserve the marker
            combined_response = format!("{} {}", combined_response.trim(), tool_result_copy);
        }

        info!(
            event = "tool_call_completed",
            function_name = %function_name,
            initial_text_length = initial_text.len(),
            final_response_chars = combined_response.len(),
            "Tool call completed successfully"
        );

        Ok(self.escape_markdown(&combined_response))
    }

    async fn handle_tool_call_only(&self, url: &str, combined_prompt: &str, images: &[ImageData], urls: &[String], function_call: &Value, discord_context: Option<&DiscordContext>) -> Result<String> {
        let function_name = function_call
            .get("name")
            .and_then(|n| n.as_str())
            .context("Missing function name in tool call")?;

        let args = function_call
            .get("args")
            .and_then(|a| a.as_object())
            .context("Missing or invalid args in tool call")?;

        info!(
            event = "tool_call_received",
            function_name = %function_name,
            args = ?args,
            "Received tool call from Gemini"
        );

        // convert args to HashMap
        let mut parameters = HashMap::new();
        for (key, value) in args {
            parameters.insert(key.clone(), value.clone());
        }

        // create tool call
        let tool_call = ToolCall {
            id: format!("call_{}", Utc::now().timestamp_nanos_opt().unwrap_or(0)),
            name: function_name.to_string(),
            parameters,
        };

        // execute the tool with Discord context if needed
        let tool_result = self.tool_executor.execute_tool(tool_call, discord_context).await;

        // For Discord tools that don't need feedback, return immediately without follow-up request
        if !self.tool_executor.tool_needs_result_feedback(function_name) {
            info!(
                event = "skipping_follow_up_for_discord_tool_only",
                function_name = %function_name,
                tool_success = tool_result.success,
                "Skipping Gemini follow-up request for Discord tool that doesn't need feedback (handle_tool_call_only)"
            );
            
            // Return appropriate response based on tool type
            let final_response = if function_name == "discord_add_reaction" {
                // Combine with tool result marker for reaction processing
                format!("{}", tool_result.result)
            } else if function_name == "discord_send_message" {
                // Combine with tool result marker for message processing
                format!("{}", tool_result.result)
            } else {
                // Other Discord tools - just return empty
                "".to_string()
            };
            
            return Ok(self.escape_markdown(&final_response));
        }

        // create the follow-up request with tool result for tools that DO need feedback
        let truncated_result = if tool_result.result.len() > 1000 && tool_result.result.contains("data:image/") {
            // for image generation, just return a short message to Gemini
            "Image generated successfully!".to_string()
        } else if function_name == "web_search" && tool_result.result.len() > 2000 {
            // Truncate long web search results to avoid API limits
            format!("{}... [truncated for length]", &tool_result.result[..2000])
        } else {
            // Tool needs feedback - send the actual result
            tool_result.result.clone()
        };
        
        let tool_response = if tool_result.success {
            json!({
                "functionResponse": {
                    "name": function_name,
                    "response": {
                        "result": truncated_result
                    }
                }
            })
        } else {
            json!({
                "functionResponse": {
                    "name": function_name,
                    "response": {
                        "error": tool_result.error.unwrap_or("Unknown error".to_string())
                    }
                }
            })
        };

        // add the original parts plus the tool response
        let mut parts = vec![json!({"text": combined_prompt})];
        
        // add images if any
        for image in images {
            parts.push(json!({
                "inline_data": {
                    "mime_type": image.mime_type,
                    "data": image.base64_data
                }
            }));
        }

        // Skip adding URLs directly to Gemini follow-up request - let the AI use fetch_web_content tool instead
        for url_str in urls {
            info!(
                event = "url_detected_in_follow_up",
                url = %url_str,
                "URL detected in follow-up - AI can use fetch_web_content tool to analyze it"
            );
        }

        // add the original function call
        parts.push(json!({
            "functionCall": function_call
        }));

        // add the tool response
        parts.push(tool_response);

        // Get tool definitions for follow-up request
        let tool_definitions = self.tool_executor.get_tool_definitions();
        
        let mut follow_up_body = json!({
            "contents": [
                {
                    "parts": parts
                }
            ]
        });
        
        // Include tools in follow-up request if available
        if !tool_definitions.is_empty() {
            follow_up_body["tools"] = json!([{
                "function_declarations": tool_definitions
            }]);
        }

        info!(
            event = "sending_follow_up_request",
            function_name = %function_name,
            tool_success = tool_result.success,
            "Sending follow-up request with tool result"
        );

        // send follow-up request
        let response = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&follow_up_body)
            .send()
            .await
            .context("Failed to send follow-up request to Gemini API")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                event = "gemini_follow_up_error",
                status_code = %status,
                error_text = %error_text,
                "Gemini API follow-up request failed"
            );
            return Err(anyhow::anyhow!(
                "Follow-up API request failed with status {}: {}",
                status,
                error_text
            ));
        }

        let response_json: Value = response
            .json()
            .await
            .context("Failed to parse follow-up JSON response from Gemini API")?;

        // Log the full response structure for debugging
        info!(
            event = "gemini_follow_up_response_debug",
            function_name = %function_name,
            response_structure = ?response_json,
            "Full Gemini follow-up response for debugging"
        );

        // Check if the follow-up response contains another function call
        if let Some(candidate) = response_json.get("candidates").and_then(|c| c.get(0)) {
            if let Some(content) = candidate.get("content") {
                if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                    // Check for function calls in the follow-up response
                    for part in parts {
                        if let Some(function_call) = part.get("functionCall") {
                            info!(
                                event = "follow_up_function_call_detected_only",
                                original_function = %function_name,
                                follow_up_function = %function_call.get("name").and_then(|n| n.as_str()).unwrap_or("unknown"),
                                "Gemini wants to make another tool call after processing the first tool result (handle_tool_call_only)"
                            );
                            
                            // Execute the follow-up function call directly
                            let new_function_name = function_call
                                .get("name")
                                .and_then(|n| n.as_str())
                                .context("Missing function name in follow-up tool call")?;

                            let new_args = function_call
                                .get("args")
                                .and_then(|a| a.as_object())
                                .context("Missing or invalid args in follow-up tool call")?;

                            // convert args to HashMap
                            let mut new_parameters = HashMap::new();
                            for (key, value) in new_args {
                                new_parameters.insert(key.clone(), value.clone());
                            }

                            // create new tool call
                            let new_tool_call = ToolCall {
                                id: format!("call_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)),
                                name: new_function_name.to_string(),
                                parameters: new_parameters,
                            };

                            // Execute the follow-up tool directly and return
                            let follow_up_result = self.tool_executor.execute_tool(new_tool_call, discord_context).await;
                            
                            if follow_up_result.success {
                                return Ok(self.escape_markdown(&follow_up_result.result));
                            } else {
                                return Err(anyhow::anyhow!(follow_up_result.error.unwrap_or("Follow-up tool execution failed".to_string())));
                            }
                        }
                    }
                }
            }
        }

        // extract the final text response - handle empty responses for Discord tools
        let final_content = if !self.tool_executor.tool_needs_result_feedback(function_name) {
            // For Discord tools that don't need feedback, empty response is expected
            if response_json.get("candidates").is_none() || 
               response_json.get("candidates").and_then(|c| c.as_array()).map(|a| a.is_empty()).unwrap_or(true) {
                info!(
                    event = "empty_response_for_discord_tool",
                    function_name = %function_name,
                    "Empty response from Gemini for Discord tool - this is expected (handle_tool_call_only)"
                );
                "" // Empty response is fine for Discord tools
            } else {
                // Try to extract text normally
                response_json
                    .get("candidates")
                    .and_then(|candidates| candidates.get(0))
                    .and_then(|candidate| candidate.get("content"))
                    .and_then(|content| content.get("parts"))
                    .and_then(|parts| parts.get(0))
                    .and_then(|part| part.get("text"))
                    .and_then(|text| text.as_str())
                    .unwrap_or("")
            }
        } else {
            // For regular tools, try to extract text content
            response_json
                .get("candidates")
                .and_then(|candidates| candidates.get(0))
                .and_then(|candidate| candidate.get("content"))
                .and_then(|content| content.get("parts"))
                .and_then(|parts| parts.get(0))
                .and_then(|part| part.get("text"))
                .and_then(|text| text.as_str())
                .unwrap_or("")
        };
            
        // for image generation, use the original tool result with image data if it was truncated
        let combined_response;
        let final_response = if function_name == "generate_image" && tool_result.result.contains("data:image/") {
            info!(
                event = "using_original_tool_result_for_image",
                original_length = tool_result.result.len(),
                gemini_response_length = final_content.len(),
                "Using original tool result instead of Gemini response for image generation (content truncated for logging)"
            );
            tool_result.result.as_str()
        } else if function_name == "discord_add_reaction" {
            info!(
                event = "discord_reaction_tool_used",
                original_tool_result = %tool_result.result,
                gemini_response = %final_content,
                "Discord reaction tool called, combining responses"
            );
            // Combine the gemini response with the original tool result marker
            combined_response = format!("{} {}", final_content.trim(), tool_result.result);
            &combined_response
        } else if function_name == "discord_send_message" {
            info!(
                event = "discord_message_tool_used",
                original_tool_result = %tool_result.result,
                gemini_response = %final_content,
                "Discord message tool called, combining responses"
            );
            // Combine the gemini response with the original tool result marker
            combined_response = format!("{} {}", final_content.trim(), tool_result.result);
            &combined_response
        } else {
            final_content
        };

        info!(
            event = "tool_call_completed",
            function_name = %function_name,
            final_response_chars = final_response.len(),
            "Tool call completed successfully (two-part response)"
        );

        Ok(self.escape_markdown(final_response))
    }

    fn format_prompt_for_display(&self, prompt: &str) -> String {
        // Extract just the current message section for logging
        if let Some(start) = prompt.find("## Current Message to Respond To:") {
            if let Some(end) = prompt[start..].find("\n\n## ") {
                // Found next section, extract just the current message part
                let message_section = &prompt[start..start + end];
                message_section.to_string()
            } else {
                // This is the last section, extract to end
                let message_section = &prompt[start..];
                message_section.trim_end().to_string()
            }
        } else {
            // Fallback - show just first 200 chars if current message section not found
            if prompt.len() > 200 {
                format!("{}... [prompt truncated - current message section not found]", &prompt[..200])
            } else {
                prompt.to_string()
            }
        }
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

    fn escape_markdown(&self, text: &str) -> String {
        let mut result = String::new();
        let mut last_end = 0;
        
        // collect all URLs and mentions and sort them by position
        let mut preservable_ranges: Vec<(usize, usize)> = Vec::new();
        
        // add URLs
        for url_match in URL_REGEX.find_iter(text) {
            preservable_ranges.push((url_match.start(), url_match.end()));
        }
        
        // add mentions (user, channel, role mentions)
        for mention_match in MENTION_REGEX.find_iter(text) {
            preservable_ranges.push((mention_match.start(), mention_match.end()));
        }
        
        // add emoticons (ASCII emoticons with special characters)
        for emoticon_match in EMOTICON_REGEX.find_iter(text) {
            preservable_ranges.push((emoticon_match.start(), emoticon_match.end()));
        }
        
        // add already-escaped characters to prevent double escaping
        for escaped_match in ESCAPED_CHAR_REGEX.find_iter(text) {
            preservable_ranges.push((escaped_match.start(), escaped_match.end()));
        }
        
        // sort by start position and merge overlapping ranges
        preservable_ranges.sort_by_key(|&(start, _)| start);
        let merged_ranges = self.merge_overlapping_ranges(preservable_ranges);
        
        // process text, preserving URLs and mentions
        for (start, end) in merged_ranges {
            // skip if this range is before our current position (shouldn't happen with merged ranges)
            if start < last_end {
                continue;
            }
            
            // escape the text before this preservable item
            let before_item = &text[last_end..start];
            result.push_str(&self.escape_markdown_chars(before_item));
            
            // add the preservable item without escaping
            result.push_str(&text[start..end]);
            
            last_end = end;
        }
        
        // escape any remaining text after the last preservable item
        let remaining = &text[last_end..];
        result.push_str(&self.escape_markdown_chars(remaining));
        
        result
    }
    
    fn merge_overlapping_ranges(&self, ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
        if ranges.is_empty() {
            return ranges;
        }
        
        let mut merged = Vec::new();
        let mut current_start = ranges[0].0;
        let mut current_end = ranges[0].1;
        
        for &(start, end) in ranges.iter().skip(1) {
            if start <= current_end {
                // overlapping or adjacent ranges, merge them
                current_end = current_end.max(end);
            } else {
                // non-overlapping range, push the current one and start a new one
                merged.push((current_start, current_end));
                current_start = start;
                current_end = end;
            }
        }
        
        // push the last range
        merged.push((current_start, current_end));
        merged
    }
    
    fn extract_image_urls(&self, text: &str) -> Vec<String> {
        IMAGE_URL_REGEX
            .find_iter(text)
            .map(|m| m.as_str().to_string())
            .collect()
    }
    
    fn extract_urls_from_message(&self, text: &str) -> Vec<String> {
        URL_REGEX
            .find_iter(text)
            .map(|m| m.as_str().to_string())
            .collect()
    }
    
    fn is_valid_web_url(&self, url: &str) -> bool {
        // only allow http/https URLs for safety and functionality
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return false;
        }
        
        // filter out URLs that Gemini doesn't support
        let unsupported_domains = [
            "tenor.com",
            "giphy.com", 
            "imgur.com/gallery",
            "reddit.com",
            "twitter.com",
            "x.com",
            "tiktok.com",
            "instagram.com",
            "facebook.com",
        ];
        
        for domain in &unsupported_domains {
            if url.contains(domain) {
                return false;
            }
        }
        
        true
    }
    
    fn escape_markdown_chars(&self, text: &str) -> String {
        text.chars()
            .map(|c| match c {
                // escape discord markdown characters
                '*' => "\\*".to_string(),
                '_' => "\\_".to_string(),
                '`' => "\\`".to_string(),
                '~' => "\\~".to_string(),
                '|' => "\\|".to_string(),
                '>' => "\\>".to_string(),
                // keep other characters as-is
                _ => c.to_string(),
            })
            .collect()
    }

    pub async fn execute_tool_with_discord_context(&self, tool_call: ToolCall, discord_context: &DiscordContext) -> crate::tools::ToolResult {
        self.tool_executor.execute_tool(tool_call, Some(discord_context)).await
    }
}
