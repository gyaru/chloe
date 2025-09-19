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

pub struct ZaiProvider {
    client: Client,
    api_key: String,
    config: ProviderConfig,
}

#[derive(Debug, Serialize)]
struct ZaiRequest {
    messages: Vec<ZaiMessage>,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ZaiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct ZaiMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ZaiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct ZaiToolCall {
    id: String,
    r#type: String,
    function: ZaiFunction,
}

#[derive(Debug, Serialize)]
struct ZaiFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct ZaiTool {
    r#type: String,
    function: ZaiToolFunction,
}

#[derive(Debug, Serialize)]
struct ZaiToolFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Deserialize)]
struct ZaiResponse {
    id: Option<String>,
    object: Option<String>,
    created: Option<u64>,
    model: Option<String>,
    choices: Vec<ZaiChoice>,
    usage: Option<ZaiUsage>,
}

#[derive(Debug, Deserialize)]
struct ZaiChoice {
    index: u32,
    message: ZaiResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZaiResponseMessage {
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<ZaiResponseToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ZaiResponseToolCall {
    id: String,
    r#type: String,
    function: ZaiResponseFunction,
}

#[derive(Debug, Deserialize)]
struct ZaiResponseFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct ZaiUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

impl ZaiProvider {
    pub fn new() -> Result<Self, LlmError> {
        let api_key = env::var("ZAI_API_KEY").map_err(|_| LlmError::AuthenticationFailed)?;

        if api_key.is_empty() {
            return Err(LlmError::AuthenticationFailed);
        }

        let default_model = env::var("LLM_MODEL").unwrap_or_else(|_| "GLM-4.5".to_string());

        let config = ProviderConfig::new("z.ai", "https://api.z.ai/api/coding/paas/v4")
            .with_default_model(&default_model)
            .with_tools_support(true)
            .with_images_support(true)
            .with_max_tokens(8192)
            .with_temperature(0.7);

        Ok(Self {
            client: Client::new(),
            api_key,
            config,
        })
    }

    fn convert_message(&self, message: &LlmMessage) -> ZaiMessage {
        let role = match message.role {
            LlmRole::System => "system",
            LlmRole::User => "user",
            LlmRole::Assistant => "assistant",
            LlmRole::Tool => "tool",
        };

        let tool_calls = message.tool_calls.as_ref().map(|calls| {
            calls
                .iter()
                .map(|call| ZaiToolCall {
                    id: call.id.clone(),
                    r#type: call.r#type.clone(),
                    function: ZaiFunction {
                        name: call.function.name.clone(),
                        arguments: call.function.arguments.clone(),
                    },
                })
                .collect()
        });

        ZaiMessage {
            role: role.to_string(),
            content: message.content.clone(),
            tool_calls,
            tool_call_id: message.tool_call_id.clone(),
            name: message.name.clone(),
        }
    }

    fn convert_tool(&self, tool: &LlmTool) -> ZaiTool {
        ZaiTool {
            r#type: tool.r#type.clone(),
            function: ZaiToolFunction {
                name: tool.function.name.clone(),
                description: tool.function.description.clone(),
                parameters: tool.function.parameters.clone(),
            },
        }
    }

    fn convert_response(&self, zai_response: ZaiResponse) -> Result<LlmResponse, LlmError> {
        info!(
            event = "zai_response_conversion_start",
            "Starting conversion of z.AI response to LlmResponse"
        );

        let choice = zai_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::ProviderError("No choices in response".to_string()))?;

        info!(
            event = "zai_response_conversion_choice",
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

        let usage = zai_response.usage.map(|u| LlmUsage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        });

        let final_response = LlmResponse {
            content: choice.message.content,
            tool_calls: tool_calls.clone(),
            finish_reason: choice.finish_reason,
            usage,
            model: zai_response.model,
        };

        info!(
            event = "zai_response_conversion_complete",
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
impl LlmProvider for ZaiProvider {
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
        vec!["GLM-4.5", "GLM-4.5-Air", "GLM-4.5V"]
    }

    async fn generate(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        // Validate model
        self.validate_model(&request.model)?;

        // Images are supported for GLM-4.5V model
        if !request.images.is_empty() && !request.model.contains("4.5V") {
            warn!(
                event = "zai_images_not_supported_for_model",
                model = %request.model,
                "Images not supported for model {} - ignoring {} images",
                request.model,
                request.images.len()
            );
        }

        let zai_messages: Vec<ZaiMessage> = request
            .messages
            .iter()
            .map(|msg| self.convert_message(msg))
            .collect();

        let zai_tools = request
            .tools
            .map(|tools| tools.iter().map(|t| self.convert_tool(t)).collect());

        let zai_request = ZaiRequest {
            messages: zai_messages,
            model: request.model.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            tools: zai_tools,
            tool_choice: request.tool_choice,
            stream: request.stream,
        };

        info!(
            event = "zai_api_request",
            model = %request.model,
            message_count = request.messages.len(),
            has_tools = zai_request.tools.is_some(),
            "Sending request to z.AI API"
        );

        // Debug the full request being sent
        info!(
            event = "zai_request_debug",
            full_request = ?zai_request,
            "Full request being sent to z.AI API"
        );

        // Debug each message in detail
        for (i, message) in zai_request.messages.iter().enumerate() {
            info!(
                event = "zai_message_debug",
                message_index = i,
                role = %message.role,
                content_length = message.content.len(),
                content_preview = %message.content.chars().take(200).collect::<String>(),
                has_tool_calls = message.tool_calls.is_some(),
                "Message details"
            );
        }

        // Debug tools if present
        if let Some(tools) = &zai_request.tools {
            for (i, tool) in tools.iter().enumerate() {
                info!(
                    event = "zai_tool_debug",
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
                .json(&zai_request)
                .send()
                .await?;

            let status = response.status();
            if status.is_success() {
                // Success - process the response
                let zai_response: ZaiResponse = response.json().await?;
                return self.convert_response(zai_response);
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
                    event = "zai_over_capacity_retry",
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
                event = "zai_api_error",
                status_code = %status,
                error_text = %error_text,
                attempt = attempt,
                "z.AI API request failed"
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
