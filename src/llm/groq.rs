use crate::llm::provider::{LlmProvider, ProviderConfig};
use crate::llm::types::{
    LlmError, LlmMessage, LlmRequest, LlmResponse, LlmRole, LlmTool, LlmToolCall, LlmUsage,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use tracing::{error, info, warn};

pub struct GroqProvider {
    client: Client,
    api_key: String,
    config: ProviderConfig,
}

#[derive(Debug, Serialize)]
struct GroqRequest {
    messages: Vec<GroqMessage>,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GroqTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    top_p: f32,
    stream: bool,
    stop: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct GroqMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<GroqToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct GroqToolCall {
    id: String,
    r#type: String,
    function: GroqFunction,
}

#[derive(Debug, Serialize)]
struct GroqFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct GroqTool {
    r#type: String,
    function: GroqToolFunction,
}

#[derive(Debug, Serialize)]
struct GroqToolFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Deserialize)]
struct GroqResponse {
    id: Option<String>,
    object: Option<String>,
    created: Option<u64>,
    model: Option<String>,
    choices: Vec<GroqChoice>,
    usage: Option<GroqUsage>,
}

#[derive(Debug, Deserialize)]
struct GroqChoice {
    index: u32,
    message: GroqResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GroqResponseMessage {
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<GroqResponseToolCall>>,
}

#[derive(Debug, Deserialize)]
struct GroqResponseToolCall {
    id: String,
    r#type: String,
    function: GroqResponseFunction,
}

#[derive(Debug, Deserialize)]
struct GroqResponseFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct GroqUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

impl GroqProvider {
    pub fn new() -> Result<Self, LlmError> {
        let api_key = env::var("GROQ_API_KEY").map_err(|_| LlmError::AuthenticationFailed)?;

        if api_key.is_empty() {
            return Err(LlmError::AuthenticationFailed);
        }

        let default_model = env::var("LLM_MODEL")
            .unwrap_or_else(|_| "moonshotai/kimi-k2-instruct-0905".to_string());

        let config = ProviderConfig::new("groq", "https://api.groq.com/openai/v1")
            .with_default_model(&default_model)
            .with_tools_support(true)
            .with_images_support(false)
            .with_max_tokens(8192)
            .with_temperature(1.8);

        Ok(Self {
            client: Client::new(),
            api_key,
            config,
        })
    }

    fn convert_message(&self, message: &LlmMessage) -> GroqMessage {
        let role = match message.role {
            LlmRole::System => "system",
            LlmRole::User => "user",
            LlmRole::Assistant => "assistant",
            LlmRole::Tool => "tool",
        };

        let tool_calls = message.tool_calls.as_ref().map(|calls| {
            calls
                .iter()
                .map(|call| GroqToolCall {
                    id: call.id.clone(),
                    r#type: call.r#type.clone(),
                    function: GroqFunction {
                        name: call.function.name.clone(),
                        arguments: call.function.arguments.clone(),
                    },
                })
                .collect()
        });

        GroqMessage {
            role: role.to_string(),
            content: message.content.clone(),
            tool_calls,
            tool_call_id: message.tool_call_id.clone(),
            name: message.name.clone(),
        }
    }

    fn convert_tool(&self, tool: &LlmTool) -> GroqTool {
        GroqTool {
            r#type: tool.r#type.clone(),
            function: GroqToolFunction {
                name: tool.function.name.clone(),
                description: tool.function.description.clone(),
                parameters: tool.function.parameters.clone(),
            },
        }
    }

    fn convert_response(&self, groq_response: GroqResponse) -> Result<LlmResponse, LlmError> {
        info!(
            event = "groq_response_conversion_start",
            "Starting conversion of Groq response to LlmResponse"
        );

        let choice = groq_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::ProviderError("No choices in response".to_string()))?;

        info!(
            event = "groq_response_conversion_choice",
            finish_reason = ?choice.finish_reason,
            content = ?choice.message.content,
            has_tool_calls = choice.message.tool_calls.is_some(),
            "Converting choice to LlmResponse"
        );

        let tool_calls = choice.message.tool_calls.map(|calls| {
            calls
                .into_iter()
                .map(|call| LlmToolCall {
                    id: call.id,
                    r#type: call.r#type,
                    function: crate::llm::types::LlmFunction {
                        name: call.function.name,
                        arguments: call.function.arguments,
                    },
                })
                .collect()
        });

        let usage = groq_response.usage.map(|u| LlmUsage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        });

        let final_response = LlmResponse {
            content: choice.message.content,
            tool_calls: tool_calls.clone(),
            finish_reason: choice.finish_reason,
            usage,
            model: groq_response.model,
        };

        info!(
            event = "groq_response_conversion_complete",
            final_content = ?final_response.content,
            final_has_tool_calls = final_response.tool_calls.is_some(),
            final_tool_calls_count = final_response.tool_calls.as_ref().map(|tc| tc.len()).unwrap_or(0),
            final_finish_reason = ?final_response.finish_reason,
            "Final LlmResponse created"
        );

        Ok(final_response)
    }
}

#[async_trait]
impl LlmProvider for GroqProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supports_tools(&self) -> bool {
        self.config.supports_tools
    }

    fn supports_images(&self) -> bool {
        self.config.supports_images
    }

    fn default_model(&self) -> &str {
        &self.config.default_model
    }

    fn available_models(&self) -> Vec<&str> {
        vec![
            "llama-3.3-70b-versatile",
            "llama-3.1-8b-instant",
            "openai/gpt-oss-120b",
            "openai/gpt-oss-20b",
            "meta-llama/llama-guard-4-12b",
            "llama-3-groq-70b-8192-tool-use-preview",
            "llama-3-groq-8b-8192-tool-use-preview",
            "moonshotai/kimi-k2-instruct-0905",
        ]
    }

    async fn generate(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        // Validate model
        self.validate_model(&request.model)?;

        // Convert images warning
        if !request.images.is_empty() {
            warn!(
                event = "groq_images_not_supported",
                "Groq does not support image inputs - ignoring {} images",
                request.images.len()
            );
        }

        let groq_messages: Vec<GroqMessage> = request
            .messages
            .iter()
            .map(|msg| self.convert_message(msg))
            .collect();

        let groq_tools = request
            .tools
            .map(|tools| tools.iter().map(|t| self.convert_tool(t)).collect());

        let groq_request = GroqRequest {
            messages: groq_messages,
            model: request.model.clone(),
            temperature: request.temperature,
            max_completion_tokens: request.max_tokens,
            tools: groq_tools,
            tool_choice: request.tool_choice,
            top_p: 1.0,
            stream: request.stream,
            stop: None,
        };

        info!(
            event = "groq_api_request",
            model = %request.model,
            message_count = request.messages.len(),
            has_tools = groq_request.tools.is_some(),
            "Sending request to Groq API"
        );

        // Debug the full request being sent
        info!(
            event = "groq_request_debug",
            full_request = ?groq_request,
            "Full request being sent to Groq API"
        );

        // Debug each message in detail
        for (i, message) in groq_request.messages.iter().enumerate() {
            info!(
                event = "groq_message_debug",
                message_index = i,
                role = %message.role,
                content_length = message.content.len(),
                content_preview = %message.content.chars().take(200).collect::<String>(),
                has_tool_calls = message.tool_calls.is_some(),
                "Message details"
            );
        }

        // Debug tools if present
        if let Some(tools) = &groq_request.tools {
            for (i, tool) in tools.iter().enumerate() {
                info!(
                    event = "groq_tool_debug",
                    tool_index = i,
                    tool_type = %tool.r#type,
                    function_name = %tool.function.name,
                    function_description = %tool.function.description,
                    "Tool definition details"
                );
            }
        }

        let url = format!("{}/chat/completions", self.config.api_base_url);

        // Retry logic with exponential backoff for over-capacity errors
        let max_retries = 3;
        let mut attempt = 0;

        loop {
            let response = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(&groq_request)
                .send()
                .await?;

            let status = response.status();
            if status.is_success() {
                // Success - process the response
                let groq_response: GroqResponse = response.json().await?;
                return self.convert_response(groq_response);
            }

            let error_text = response.text().await.unwrap_or_default();

            // Check if this is an over-capacity error and we haven't exceeded max retries
            let is_over_capacity = status == 503
                && (error_text.contains("over capacity")
                    || error_text.contains("Please try again"));

            if is_over_capacity && attempt < max_retries {
                attempt += 1;
                let delay_ms = 1000_u64 * (2_u64.pow(attempt as u32 - 1)); // Exponential backoff: 1s, 2s, 4s

                warn!(
                    event = "groq_over_capacity_retry",
                    attempt = attempt,
                    max_retries = max_retries,
                    delay_ms = delay_ms,
                    "Model is over capacity, retrying with exponential backoff"
                );

                tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                continue;
            }

            // Non-recoverable error or max retries exceeded
            error!(
                event = "groq_api_error",
                status_code = %status,
                error_text = %error_text,
                attempt = attempt,
                "Groq API request failed"
            );

            return Err(match status.as_u16() {
                401 => LlmError::AuthenticationFailed,
                429 => LlmError::RateLimitExceeded,
                400 => LlmError::InvalidRequest(error_text),
                _ => LlmError::ApiError {
                    status: status.as_u16(),
                    message: error_text,
                },
            });
        }
    }

    fn get_config(&self) -> ProviderConfig {
        self.config.clone()
    }
}
