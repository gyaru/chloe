use crate::settings::Settings;
use crate::tools::{tool_definitions::*, tool_executor::ToolExecutor, ToolCall};
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
        let (response, _) = self.prompt_with_context_and_sender(
            context, 
            None::<fn(String) -> std::future::Ready<()>>,
            None::<fn() -> std::future::Ready<()>>
        ).await?;
        Ok(self.escape_markdown(&response))
    }

    pub async fn prompt_with_context_and_sender<F, Fut, T, TFut>(
        &self, 
        context: ConversationContext, 
        message_sender: Option<F>,
        typing_starter: Option<T>
    ) -> Result<(String, bool)> 
    where
        F: FnOnce(String) -> Fut + Send,
        Fut: std::future::Future<Output = ()> + Send,
        T: FnOnce() -> TFut + Send,
        TFut: std::future::Future<Output = ()> + Send,
    {
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

        self.send_request_with_images_and_sender(&url, &combined_prompt, &context.current_images, message_sender, typing_starter)
            .await
    }

    fn enrich_system_prompt_with_context(
        &self,
        base_prompt: &str,
        context: &ConversationContext,
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
            
            enriched.push_str("\n**IMPORTANT**: When a user asks you to search for something, look something up, find information, or requests current/recent data, you MUST use the appropriate tool. Don't just acknowledge the request - actually perform the search or calculation using the tools above. \n\nExamples of when to use web_search:\n- \"search for [anything]\"\n- \"find me [anything]\"\n- \"look up [anything]\"\n- \"can you search for [anything]\"\n- Any request for music, videos, news, products, current events, etc.\n\nAlways use tools when users make requests that these tools can fulfill.\n");
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

        info!(
            event = "gemini_raw_response",
            response = ?response_json,
            "Raw response from Gemini API for debugging"
        );

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
                                let response = self.handle_tool_call_only(url, combined_prompt, images, function_call).await?;
                                return Ok((self.escape_markdown(&response), true)); // true = initial message was sent
                            } else {
                                // original combined response behavior - start typing for tool execution
                                if let Some(typing) = typing_starter {
                                    typing().await;
                                }
                                
                                let response = self.handle_tool_call(url, combined_prompt, images, function_call, &initial_text).await?;
                                return Ok((self.escape_markdown(&response), false)); // false = no initial message sent
                            }
                        }
                    }

                    // no tool calls, return the text response
                    if !initial_text.is_empty() {
                        info!(
                            event = "gemini_api_response",
                            response_chars = initial_text.len(),
                            estimated_tokens = self.estimate_tokens(&initial_text),
                            response = %self.format_response_for_display(&initial_text),
                            "Received text response from Gemini API"
                        );
                        return Ok((self.escape_markdown(&initial_text), false)); // false = no initial message sent (single response)
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

    async fn handle_tool_call(&self, url: &str, combined_prompt: &str, images: &[ImageData], function_call: &Value, initial_text: &str) -> Result<String> {
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

        // Execute the tool
        let tool_result = self.tool_executor.execute_tool(tool_call).await;

        // Create the follow-up request with tool result
        let tool_response = if tool_result.success {
            json!({
                "functionResponse": {
                    "name": function_name,
                    "response": {
                        "result": tool_result.result
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

        // add the original function call
        parts.push(json!({
            "functionCall": function_call
        }));

        // add the tool response
        parts.push(tool_response);

        let follow_up_body = json!({
            "contents": [
                {
                    "parts": parts
                }
            ]
        });

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

        // extract the final text response
        let final_content = response_json
            .get("candidates")
            .and_then(|candidates| candidates.get(0))
            .and_then(|candidate| candidate.get("content"))
            .and_then(|content| content.get("parts"))
            .and_then(|parts| parts.get(0))
            .and_then(|part| part.get("text"))
            .and_then(|text| text.as_str())
            .context("Failed to extract text from follow-up response")?;

        // combine initial text with tool result if initial text exists and is substantial
        let combined_response = if !initial_text.trim().is_empty() && initial_text.trim().len() > 10 {
            // if the final content already contains the initial text, don't duplicate
            if final_content.contains(initial_text.trim()) {
                final_content.to_string()
            } else {
                format!("{}\n\n{}", initial_text.trim(), final_content)
            }
        } else {
            final_content.to_string()
        };

        info!(
            event = "tool_call_completed",
            function_name = %function_name,
            initial_text_length = initial_text.len(),
            final_response_chars = combined_response.len(),
            "Tool call completed successfully"
        );

        Ok(self.escape_markdown(&combined_response))
    }

    async fn handle_tool_call_only(&self, url: &str, combined_prompt: &str, images: &[ImageData], function_call: &Value) -> Result<String> {
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

        // execute the tool
        let tool_result = self.tool_executor.execute_tool(tool_call).await;

        // create the follow-up request with tool result
        let tool_response = if tool_result.success {
            json!({
                "functionResponse": {
                    "name": function_name,
                    "response": {
                        "result": tool_result.result
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

        // add the original function call
        parts.push(json!({
            "functionCall": function_call
        }));

        // add the tool response
        parts.push(tool_response);

        let follow_up_body = json!({
            "contents": [
                {
                    "parts": parts
                }
            ]
        });

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

        // extract the final text response
        let final_content = response_json
            .get("candidates")
            .and_then(|candidates| candidates.get(0))
            .and_then(|candidate| candidate.get("content"))
            .and_then(|content| content.get("parts"))
            .and_then(|parts| parts.get(0))
            .and_then(|part| part.get("text"))
            .and_then(|text| text.as_str())
            .context("Failed to extract text from follow-up response")?;

        info!(
            event = "tool_call_completed",
            function_name = %function_name,
            final_response_chars = final_content.len(),
            "Tool call completed successfully (two-part response)"
        );

        Ok(self.escape_markdown(final_content))
    }

    fn format_prompt_for_display(&self, prompt: &str) -> String {
        // find where the dynamic content starts (after the base prompt)
        if let Some(dynamic_start) = prompt.find("\n\n## Current Date & Time") {
            let base_prompt = &prompt[..dynamic_start];
            let dynamic_content = &prompt[dynamic_start..];
            
            // show just a snippet of the base prompt + all dynamic content
            let base_snippet = if base_prompt.len() > 100 {
                format!("{}... [base prompt truncated for display]", &base_prompt[..100])
            } else {
                base_prompt.to_string()
            };
            
            format!("{}{}", base_snippet, dynamic_content)
        } else {
            // fallback to original behavior if no dynamic content found
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
        
        // find all URLs and preserve them without escaping
        for url_match in URL_REGEX.find_iter(text) {
            // escape the text before this URL
            let before_url = &text[last_end..url_match.start()];
            result.push_str(&self.escape_markdown_chars(before_url));
            
            // add the URL without escaping
            result.push_str(url_match.as_str());
            
            last_end = url_match.end();
        }
        
        // escape any remaining text after the last URL
        let remaining = &text[last_end..];
        result.push_str(&self.escape_markdown_chars(remaining));
        
        result
    }
    
    fn escape_markdown_chars(&self, text: &str) -> String {
        text.chars()
            .map(|c| match c {
                // escape Discord markdown characters
                '*' => "\\*".to_string(),
                '_' => "\\_".to_string(),
                '`' => "\\`".to_string(),
                '~' => "\\~".to_string(),
                '|' => "\\|".to_string(),
                '>' => "\\>".to_string(),
                // Keep other characters as-is
                _ => c.to_string(),
            })
            .collect()
    }
}
