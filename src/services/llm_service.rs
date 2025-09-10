use crate::services::gemini_types::{
    self, FunctionCall, FunctionResponse, FunctionResponseData, GeminiRequest,
    GeminiResponse,
};
use crate::services::prompt_builder::PromptBuilder;
use crate::settings::Settings;
use crate::tools::{
    DiscordAddReactionTool, DiscordContext, DiscordSendMessageTool, ToolCall, ToolName, ToolResult, WebSearchTool,
    tool_executor::ToolExecutor,
};
use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::Client;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    env,
    sync::Arc,
};
use tokio::sync::RwLock;
use tracing::{error, info};
use crate::utils::regex_patterns::{
    URL_REGEX, IMAGE_URL_REGEX, MENTION_REGEX, EMOTICON_REGEX, ESCAPED_CHAR_REGEX
};

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
    pub is_random_reply: bool,
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
    rate_limiter: Arc<crate::utils::RateLimiter>,
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
        tool_executor.register_tool(Arc::new(crate::tools::FetchTool::new()));
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
            rate_limiter: Arc::new(crate::utils::create_llm_rate_limiter()),
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
        let response = self
            .prompt_with_context_and_sender(
                context,
                None::<fn(String) -> std::future::Ready<()>>,
                None::<fn() -> std::future::Ready<()>>,
            )
            .await?;
        Ok(self.escape_markdown(&response.text))
    }

    pub async fn prompt_with_context_and_sender<F, Fut, T, TFut>(
        &self,
        context: ConversationContext,
        message_sender: Option<F>,
        typing_starter: Option<T>,
    ) -> Result<LlmResponse>
    where
        F: FnOnce(String) -> Fut + Send,
        Fut: std::future::Future<Output = ()> + Send,
        T: FnOnce() -> TFut + Send,
        TFut: std::future::Future<Output = ()> + Send,
    {
        self.prompt_with_context_and_sender_with_discord(
            context,
            message_sender,
            typing_starter,
            None,
        )
        .await
    }

    pub async fn prompt_with_context_and_sender_with_discord<F, Fut, T, TFut>(
        &self,
        context: ConversationContext,
        message_sender: Option<F>,
        typing_starter: Option<T>,
        discord_context: Option<&DiscordContext>,
    ) -> Result<LlmResponse>
    where
        F: FnOnce(String) -> Fut + Send,
        Fut: std::future::Future<Output = ()> + Send,
        T: FnOnce() -> TFut + Send,
        TFut: std::future::Future<Output = ()> + Send,
    {
        let global_settings = self.settings.get_global_settings().await;

        let enriched_system_prompt = self
            .enrich_system_prompt_with_context(&global_settings.prompt, &context, discord_context)
            .await;

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

        let (text, initial_sent) = self
            .send_request_with_images_urls_and_sender(
                &url,
                &combined_prompt,
                &context.current_images,
                &message_urls,
                message_sender,
                typing_starter,
                discord_context,
            )
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
        let tool_definitions = self.tool_executor.get_tool_definitions();
        let prompt_builder = PromptBuilder::new(base_prompt.to_string(), tool_definitions);
        prompt_builder.build_enriched_prompt(context, discord_context).await
    }

    fn estimate_tokens(&self, text: &str) -> usize {
        (text.len() as f32 / 4.0).ceil() as usize
    }

    async fn send_request(&self, url: &str, combined_prompt: &str) -> Result<String> {
        self.send_request_with_images(url, combined_prompt, &[])
            .await
    }

    async fn send_request_with_images(
        &self,
        url: &str,
        combined_prompt: &str,
        images: &[ImageData],
    ) -> Result<String> {
        let (response, _) = self
            .send_request_with_images_and_sender(
                url,
                combined_prompt,
                images,
                None::<fn(String) -> std::future::Ready<()>>,
                None::<fn() -> std::future::Ready<()>>,
            )
            .await?;
        Ok(self.escape_markdown(&response))
    }

    async fn send_request_with_images_and_sender<F, Fut, T, TFut>(
        &self,
        url: &str,
        combined_prompt: &str,
        images: &[ImageData],
        message_sender: Option<F>,
        typing_starter: Option<T>,
    ) -> Result<(String, bool)>
    where
        F: FnOnce(String) -> Fut + Send,
        Fut: std::future::Future<Output = ()> + Send,
        T: FnOnce() -> TFut + Send,
        TFut: std::future::Future<Output = ()> + Send,
    {
        self.send_request_with_images_urls_and_sender(
            url,
            combined_prompt,
            images,
            &[],
            message_sender,
            typing_starter,
            None,
        )
        .await
    }

    async fn send_request_with_images_urls_and_sender<F, Fut, T, TFut>(
        &self,
        url: &str,
        combined_prompt: &str,
        images: &[ImageData],
        urls: &[String],
        message_sender: Option<F>,
        typing_starter: Option<T>,
        discord_context: Option<&DiscordContext>,
    ) -> Result<(String, bool)>
    where
        F: FnOnce(String) -> Fut + Send,
        Fut: std::future::Future<Output = ()> + Send,
        T: FnOnce() -> TFut + Send,
        TFut: std::future::Future<Output = ()> + Send,
    {
        // Skip adding URLs directly to Gemini request - let the AI use fetch tool instead
        for url_str in urls {
            info!(
                event = "url_detected_in_message",
                url = %url_str,
                "URL detected in message - AI can use fetch tool to analyze it"
            );
        }

        // Build typed request
        let tool_definitions = self.tool_executor.get_tool_definitions();
        let request = GeminiRequest::new(combined_prompt)
            .with_images(images)
            .with_tools(tool_definitions)
            .with_safety_settings(gemini_types::default_safety_settings());

        info!(
            event = "gemini_api_request",
            model = "gemini-2.5-flash-preview-05-20",
            prompt_chars = combined_prompt.len(),
            estimated_tokens = self.estimate_tokens(combined_prompt),
            prompt = %self.format_prompt_for_display(combined_prompt),
            "Sending request to Gemini API"
        );

        // Apply rate limiting
        let rate_limit_key = if let Some(discord_ctx) = discord_context {
            format!("llm_channel_{}", discord_ctx.channel_id)
        } else {
            "llm_general".to_string()
        };
        
        let _permit = match self.rate_limiter.acquire(rate_limit_key).await {
            Ok(permit) => permit,
            Err(_) => {
                error!(event = "rate_limit_timeout", "Failed to acquire rate limit permit");
                return Err(anyhow::anyhow!("Rate limit timeout"));
            }
        };

        // Retry logic for transient errors
        let mut retry_count = 0;
        let max_retries = 3;
        let mut last_error = None;

        let response = loop {
            let response = self
                .client
                .post(url)
                .header("Content-Type", "application/json")
                .json(&request)
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

        let response_json: GeminiResponse = response
            .json()
            .await
            .context("Failed to parse JSON response from Gemini API")?;

        // Log response structure
        info!(
            event = "gemini_raw_response",
            has_candidates = response_json.candidates.is_some(),
            is_blocked = response_json.is_blocked(),
            "Raw response from Gemini API"
        );

        // check if the response was blocked for safety reasons
        if response_json.is_blocked() {
            let block_reason = response_json.get_block_reason().unwrap_or("UNKNOWN");
            let safety_message = match block_reason {
                "SAFETY" => {
                    "Oh no! I can't respond to that because it might involve harmful content. Let's talk about something else instead! ✨"
                }
                "OTHER" => {
                    "Hmm, I'm not able to respond to that right now. Maybe we could try a different topic? 💭"
                }
                _ => {
                    "Something's preventing me from responding to that. Want to try asking something else? 🤔"
                }
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
                    safety_params.insert(
                        "content".to_string(),
                        serde_json::Value::String(safety_message.to_string()),
                    );
                    safety_params.insert(
                        "reply_to_original".to_string(),
                        serde_json::Value::Bool(true),
                    );

                    let safety_tool_call = crate::tools::ToolCall {
                        id: format!(
                            "safety_{}",
                            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
                        ),
                        name: ToolName::DiscordSendMessage.as_str().to_string(),
                        parameters: safety_params,
                    };

                    // Execute the Discord message tool directly
                    let _result = self
                        .tool_executor
                        .execute_tool(safety_tool_call, Some(discord_ctx))
                        .await;

                    // Return empty response since we handled it directly
                    return Ok(("".to_string(), false));
                }

                // Fallback for when no Discord context (shouldn't happen in Discord usage)
                return Ok((safety_message.to_string(), false));
        }

        // check if the response contains tool calls
        let initial_text = response_json.get_text().unwrap_or("").to_string();
        
        if let Some(function_call) = response_json.get_function_call() {
            // Convert FunctionCall to Value for backward compatibility
            let function_call_value = serde_json::to_value(function_call)
                .context("Failed to convert function call to Value")?;
            
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
                let response = self
                    .handle_tool_call_only(
                        url,
                        combined_prompt,
                        images,
                        urls,
                        &function_call_value,
                        discord_context,
                    )
                    .await?;
                return Ok((self.escape_markdown(&response), true)); // true = initial message was sent
            } else {
                // original combined response behavior - start typing for tool execution
                if let Some(typing) = typing_starter {
                    typing().await;
                }

                let response = self
                    .handle_tool_call(
                        url,
                        combined_prompt,
                        images,
                        urls,
                        &function_call_value,
                        &initial_text,
                        discord_context,
                    )
                    .await?;
                return Ok((self.escape_markdown(&response), false)); // false = no initial message sent
            }
        } else if response_json.has_text() && !response_json.has_function_call() {
            // no tool calls - this should not happen with our tool-only requirement
            error!(
                event = "gemini_raw_text_response",
                response_chars = initial_text.len(),
                response = %self.format_response_for_display(&initial_text),
                "Gemini returned raw text instead of using tools - this violates our tool-only requirement"
            );

            // Autocorrect by sending the raw text via discord_send_message
            if let Some(discord_ctx) = discord_context {
                error!(
                    event = "gemini_tool_violation_autocorrected",
                    text_length = initial_text.len(),
                    raw_text = %initial_text,
                    "Gemini violated tool-only requirement. Automatically sending raw text via discord_send_message"
                );
                
                let mut message_params = HashMap::new();
                message_params.insert("content".to_string(), json!(initial_text));
                message_params.insert("reply_to_original".to_string(), json!(true));

                let autocorrect_tool_call = ToolCall {
                    id: format!("autocorrect_{}", Utc::now().timestamp_nanos_opt().unwrap_or(0)),
                    name: "discord_send_message".to_string(),
                    parameters: message_params,
                };

                let _result = self
                    .tool_executor
                    .execute_tool(autocorrect_tool_call, Some(discord_ctx))
                    .await;
            }
            
            return Ok(("".to_string(), false));
        }

        error!(
            event = "failed_to_parse_gemini_response",
            response_structure = ?response_json,
            "Failed to extract response from Gemini API - full response structure logged"
        );

        Err(anyhow::anyhow!(
            "Failed to extract response from Gemini API. Response structure: {}",
            serde_json::to_string_pretty(&response_json).unwrap_or_default()
        ))
    }

    async fn handle_tool_call(
        &self,
        url: &str,
        combined_prompt: &str,
        images: &[ImageData],
        urls: &[String],
        function_call: &Value,
        initial_text: &str,
        discord_context: Option<&DiscordContext>,
    ) -> Result<String> {
        // Execute up to 5 tool calls in sequence
        self.handle_tool_call_generic(
            url,
            combined_prompt,
            images,
            urls,
            function_call,
            Some(initial_text),
            discord_context,
            5,
        )
        .await
    }

    // Removed handle_tool_call_chain - replaced by handle_tool_call_generic

    async fn handle_tool_call_only(
        &self,
        url: &str,
        combined_prompt: &str,
        images: &[ImageData],
        urls: &[String],
        function_call: &Value,
        discord_context: Option<&DiscordContext>,
    ) -> Result<String> {
        // Execute up to 5 tool calls in sequence
        self.handle_tool_call_generic(
            url,
            combined_prompt,
            images,
            urls,
            function_call,
            None,
            discord_context,
            5,
        )
        .await
    }

    // Removed handle_tool_call_only_chain - replaced by handle_tool_call_generic

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
                format!(
                    "{}... [prompt truncated - current message section not found]",
                    &prompt[..200]
                )
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

    pub async fn execute_tool_with_discord_context(
        &self,
        tool_call: ToolCall,
        discord_context: &DiscordContext,
    ) -> crate::tools::ToolResult {
        self.tool_executor
            .execute_tool(tool_call, Some(discord_context))
            .await
    }

    // Unified handler for both tool call scenarios
    async fn handle_tool_call_generic(
        &self,
        url: &str,
        combined_prompt: &str,
        images: &[ImageData],
        urls: &[String],
        function_call: &Value,
        initial_text: Option<&str>,
        discord_context: Option<&DiscordContext>,
        max_calls: usize,
    ) -> Result<String> {
        // Extract tool name and args
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
            has_initial_text = initial_text.is_some(),
            "Received tool call from Gemini"
        );

        // Convert args to HashMap
        let mut parameters = HashMap::new();
        for (key, value) in args {
            parameters.insert(key.clone(), value.clone());
        }

        // Create tool call
        let tool_call = ToolCall {
            id: format!("call_{}", Utc::now().timestamp_nanos_opt().unwrap_or(0)),
            name: function_name.to_string(),
            parameters,
        };

        // Execute the tool
        let tool_result = self
            .tool_executor
            .execute_tool(tool_call, discord_context)
            .await;

        // For Discord tools that don't need feedback, return immediately
        if !self.tool_executor.tool_needs_result_feedback(function_name) {
            info!(
                event = "skipping_follow_up_for_discord_tool",
                function_name = %function_name,
                tool_success = tool_result.success,
                "Skipping Gemini follow-up request for Discord tool that doesn't need feedback"
            );

            return if let Some(initial_text) = initial_text {
                if !initial_text.trim().is_empty() {
                    // Combine initial text with tool result for certain tools
                    let combined = if matches!(
                        ToolName::from_str(function_name).ok(),
                        Some(ToolName::DiscordAddReaction | ToolName::DiscordSendMessage)
                    )
                    {
                        format!("{} {}", initial_text.trim(), tool_result.result)
                    } else {
                        initial_text.to_string()
                    };
                    Ok(self.escape_markdown(&combined))
                } else {
                    Ok("".to_string())
                }
            } else {
                // No initial text case - for tool_call_only scenario
                let final_response = if matches!(
                    ToolName::from_str(function_name).ok(),
                    Some(ToolName::DiscordAddReaction | ToolName::DiscordSendMessage)
                )
                {
                    tool_result.result
                } else {
                    "".to_string()
                };
                Ok(self.escape_markdown(&final_response))
            };
        }

        // Build follow-up request for tools that need feedback
        let follow_up_response = self
            .send_tool_follow_up_request(
                url,
                combined_prompt,
                images,
                urls,
                function_call,
                function_name,
                &tool_result,
            )
            .await?;

        // Process the follow-up response
        self.process_tool_follow_up_response(
            &follow_up_response,
            url,
            combined_prompt,
            images,
            urls,
            initial_text,
            function_name,
            &tool_result,
            discord_context,
            max_calls,
        )
        .await
    }

    // Helper to send follow-up request with tool result
    async fn send_tool_follow_up_request(
        &self,
        url: &str,
        combined_prompt: &str,
        images: &[ImageData],
        _urls: &[String],
        function_call: &Value,
        function_name: &str,
        tool_result: &ToolResult,
    ) -> Result<GeminiResponse> {
        // Prepare truncated result for certain tools
        let truncated_result = self.prepare_tool_result_for_follow_up(function_name, &tool_result.result);

        // Create typed function response
        let function_response = FunctionResponse {
            name: function_name.to_string(),
            response: if tool_result.success {
                FunctionResponseData {
                    result: Some(truncated_result),
                    error: None,
                }
            } else {
                FunctionResponseData {
                    result: None,
                    error: Some(tool_result.error.as_deref().unwrap_or("Unknown error").to_string()),
                }
            },
        };

        // Convert function_call Value to FunctionCall
        let function_call_typed: FunctionCall = serde_json::from_value(function_call.clone())
            .context("Failed to parse function call")?;

        // Build typed request
        let tool_definitions = self.tool_executor.get_tool_definitions();
        let request = GeminiRequest::new(combined_prompt)
            .with_images(images)
            .add_function_call_parts(&function_call_typed, function_response)
            .with_tools(tool_definitions)
            .with_safety_settings(gemini_types::default_safety_settings());

        info!(
            event = "sending_follow_up_request",
            function_name = %function_name,
            tool_success = tool_result.success,
            "Sending follow-up request with tool result"
        );

        // Send the request
        let response = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&request)
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

        response
            .json()
            .await
            .context("Failed to parse follow-up JSON response from Gemini API")
    }

    // Helper to process follow-up response
    async fn process_tool_follow_up_response(
        &self,
        response_json: &GeminiResponse,
        url: &str,
        combined_prompt: &str,
        images: &[ImageData],
        urls: &[String],
        initial_text: Option<&str>,
        function_name: &str,
        tool_result: &ToolResult,
        discord_context: Option<&DiscordContext>,
        max_calls: usize,
    ) -> Result<String> {
        info!(
            event = "gemini_follow_up_response_debug",
            function_name = %function_name,
            response_structure = ?response_json,
            "Full Gemini follow-up response for debugging"
        );

        // Check if response contains another function call
        if let Some(next_function_call) = response_json.get_function_call() {
            let function_call_value = serde_json::to_value(next_function_call)
                .context("Failed to convert function call to Value")?;
            
            return self.handle_follow_up_tool_call(
                &function_call_value,
                url,
                combined_prompt,
                images,
                urls,
                initial_text,
                discord_context,
                max_calls,
            )
            .await;
        }

        // Check for raw text violation
        if response_json.has_text() && !response_json.has_function_call() {
            if let Some(raw_text) = response_json.get_text() {
                return self
                    .handle_raw_text_violation(raw_text, discord_context)
                    .await;
            }
        }

        // Extract final response
        let final_content = self.extract_final_content_from_response(
            response_json,
            function_name,
        );

        // Combine with initial text if present
        self.combine_final_response(
            &final_content,
            initial_text,
            function_name,
            &tool_result.result,
        )
    }

    // Helper for handling follow-up tool calls
    async fn handle_follow_up_tool_call(
        &self,
        next_function_call: &Value,
        url: &str,
        combined_prompt: &str,
        images: &[ImageData],
        urls: &[String],
        initial_text: Option<&str>,
        discord_context: Option<&DiscordContext>,
        max_calls: usize,
    ) -> Result<String> {
        let next_function_name = next_function_call
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unknown");

        info!(
            event = "follow_up_function_call_detected",
            follow_up_function = %next_function_name,
            remaining_calls = max_calls - 1,
            "Gemini wants to make another tool call"
        );

        if max_calls <= 1 {
            error!(
                event = "max_tool_calls_reached",
                "Maximum number of tool calls reached (5), stopping chain"
            );

            if let Some(discord_ctx) = discord_context {
                self.send_tool_limit_reminder(discord_ctx).await;
            }
            return Ok("".to_string());
        }

        // Recursively handle the next tool call
        Box::pin(self.handle_tool_call_generic(
            url,
            combined_prompt,
            images,
            urls,
            next_function_call,
            initial_text,
            discord_context,
            max_calls - 1,
        ))
        .await
    }

    // Helper to check for raw text violations
    fn check_for_raw_text_violation(&self, parts: &[Value]) -> Option<String> {
        let has_text = parts.iter().any(|part| part.get("text").is_some());
        let has_function_call = parts.iter().any(|part| part.get("functionCall").is_some());

        if has_text && !has_function_call {
            parts
                .iter()
                .find_map(|part| part.get("text").and_then(|t| t.as_str()))
                .map(|s| s.to_string())
        } else {
            None
        }
    }

    // Helper to handle raw text violations
    async fn handle_raw_text_violation(
        &self,
        raw_text: &str,
        discord_context: Option<&DiscordContext>,
    ) -> Result<String> {
        if let Some(discord_ctx) = discord_context {
            error!(
                event = "gemini_tool_violation_autocorrected",
                text_length = raw_text.len(),
                raw_text = %raw_text,
                "Gemini violated tool-only requirement. Automatically sending raw text via discord_send_message"
            );

            let mut message_params = HashMap::new();
            message_params.insert("content".to_string(), json!(raw_text));
            message_params.insert("reply_to_original".to_string(), json!(true));

            let autocorrect_tool_call = ToolCall {
                id: format!("autocorrect_{}", Utc::now().timestamp_nanos_opt().unwrap_or(0)),
                name: "discord_send_message".to_string(),
                parameters: message_params,
            };

            let _result = self
                .tool_executor
                .execute_tool(autocorrect_tool_call, Some(discord_ctx))
                .await;
        }

        Ok("".to_string())
    }

    // Helper to send tool limit reminder
    async fn send_tool_limit_reminder(&self, discord_ctx: &DiscordContext) {
        let mut reminder_params = HashMap::new();
        reminder_params.insert(
            "content".to_string(),
            json!("I've reached my tool call limit. Please use the discord_send_message tool to continue the conversation!"),
        );
        reminder_params.insert("reply_to_original".to_string(), json!(true));

        let reminder_tool_call = ToolCall {
            id: format!("reminder_{}", Utc::now().timestamp_nanos_opt().unwrap_or(0)),
            name: "discord_send_message".to_string(),
            parameters: reminder_params,
        };

        let _result = self
            .tool_executor
            .execute_tool(reminder_tool_call, Some(discord_ctx))
            .await;
    }

    // Helper to prepare tool results for follow-up
    fn prepare_tool_result_for_follow_up(&self, function_name: &str, result: &str) -> String {
        if result.len() > 1000 && result.contains("data:image/") {
            "Image generated successfully!".to_string()
        } else if matches!(ToolName::from_str(function_name).ok(), Some(ToolName::WebSearch)) && result.len() > 2000 {
            format!("{}... [truncated for length]", &result[..2000])
        } else {
            result.to_string()
        }
    }

    // Helper to extract final content from response
    fn extract_final_content_from_response(
        &self,
        response_json: &GeminiResponse,
        function_name: &str,
    ) -> String {
        if !self.tool_executor.tool_needs_result_feedback(function_name) {
            // For Discord tools that don't need feedback, empty response is expected
            if response_json.candidates.is_none()
                || response_json
                    .candidates
                    .as_ref()
                    .map(|c| c.is_empty())
                    .unwrap_or(true)
            {
                info!(
                    event = "empty_response_for_discord_tool",
                    function_name = %function_name,
                    "Empty response from Gemini for Discord tool - this is expected"
                );
                return "".to_string();
            }
        }

        // Use the typed helper method to extract text content
        response_json.get_text().unwrap_or("").to_string()
    }

    // Helper to combine final response
    fn combine_final_response(
        &self,
        final_content: &str,
        initial_text: Option<&str>,
        function_name: &str,
        tool_result: &str,
    ) -> Result<String> {
        let combined = if let Some(initial_text) = initial_text {
            if !initial_text.trim().is_empty() && initial_text.trim().len() > 10 {
                if final_content.contains(initial_text.trim()) {
                    final_content.to_string()
                } else {
                    format!("{}\n\n{}", initial_text.trim(), final_content)
                }
            } else {
                final_content.to_string()
            }
        } else {
            final_content.to_string()
        };

        // Special handling for certain tools
        let final_response = if matches!(ToolName::from_str(function_name).ok(), Some(ToolName::GenerateImage)) && tool_result.contains("data:image/") {
            info!(
                event = "using_original_tool_result_for_image",
                "Using original tool result instead of Gemini response for image generation"
            );
            tool_result.to_string()
        } else if matches!(
            ToolName::from_str(function_name).ok(),
            Some(ToolName::DiscordAddReaction | ToolName::DiscordSendMessage)
        ) {
            info!(
                event = "discord_tool_used",
                tool_name = %function_name,
                "Discord tool called, combining responses"
            );
            format!("{} {}", combined.trim(), tool_result)
        } else {
            combined
        };

        Ok(self.escape_markdown(&final_response))
    }
}
