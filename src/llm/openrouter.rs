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

pub struct OpenRouterProvider {
    client: Client,
    api_key: String,
    config: ProviderConfig,
}

#[derive(Debug, Serialize)]
struct OpenRouterRequest {
    model: String,
    messages: Vec<OpenRouterMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenRouterTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct OpenRouterMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenRouterToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct OpenRouterToolCall {
    id: String,
    r#type: String,
    function: OpenRouterFunction,
}

#[derive(Debug, Serialize)]
struct OpenRouterFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct OpenRouterTool {
    r#type: String,
    function: OpenRouterToolFunction,
}

#[derive(Debug, Serialize)]
struct OpenRouterToolFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponse {
    id: Option<String>,
    object: Option<String>,
    created: Option<u64>,
    model: Option<String>,
    choices: Vec<OpenRouterChoice>,
    usage: Option<OpenRouterUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChoice {
    index: u32,
    message: OpenRouterResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponseMessage {
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<OpenRouterResponseToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponseToolCall {
    id: String,
    r#type: String,
    function: OpenRouterResponseFunction,
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponseFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenRouterUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

impl OpenRouterProvider {
    pub fn new() -> Result<Self, LlmError> {
        let api_key = env::var("OPENROUTER_API_KEY").map_err(|_| LlmError::AuthenticationFailed)?;

        if api_key.is_empty() {
            return Err(LlmError::AuthenticationFailed);
        }

        let default_model =
            env::var("LLM_MODEL").unwrap_or_else(|_| "openai/gpt-4o-mini".to_string());

        let config = ProviderConfig::new("openrouter", "https://openrouter.ai/api/v1")
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

    fn convert_message(&self, message: &LlmMessage) -> OpenRouterMessage {
        let role = match message.role {
            LlmRole::System => "system",
            LlmRole::User => "user",
            LlmRole::Assistant => "assistant",
            LlmRole::Tool => "tool",
        };

        let tool_calls = message.tool_calls.as_ref().map(|calls| {
            calls
                .iter()
                .map(|call| OpenRouterToolCall {
                    id: call.id.clone(),
                    r#type: call.r#type.clone(),
                    function: OpenRouterFunction {
                        name: call.function.name.clone(),
                        arguments: call.function.arguments.clone(),
                    },
                })
                .collect()
        });

        OpenRouterMessage {
            role: role.to_string(),
            content: message.content.clone(),
            tool_calls,
            tool_call_id: message.tool_call_id.clone(),
            name: message.name.clone(),
        }
    }

    fn convert_tool(&self, tool: &LlmTool) -> OpenRouterTool {
        OpenRouterTool {
            r#type: tool.r#type.clone(),
            function: OpenRouterToolFunction {
                name: tool.function.name.clone(),
                description: tool.function.description.clone(),
                parameters: tool.function.parameters.clone(),
            },
        }
    }

    fn convert_response(
        &self,
        openrouter_response: OpenRouterResponse,
    ) -> Result<LlmResponse, LlmError> {
        info!(
            event = "openrouter_response_conversion_start",
            "Starting conversion of OpenRouter response to LlmResponse"
        );

        let choice = openrouter_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::ProviderError("No choices in response".to_string()))?;

        info!(
            event = "openrouter_response_conversion_choice",
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

        let usage = openrouter_response.usage.map(|u| LlmUsage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        });

        let final_response = LlmResponse {
            content: choice.message.content,
            tool_calls: tool_calls.clone(),
            finish_reason: choice.finish_reason,
            usage,
            model: openrouter_response.model,
        };

        info!(
            event = "openrouter_response_conversion_complete",
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
impl LlmProvider for OpenRouterProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supports_tools(&self) -> bool {
        // Tool support depends on the model - most modern models support it
        self.config.supports_tools
    }

    fn supports_images(&self) -> bool {
        // Image support depends on the model - vision models support it
        self.config.supports_images
    }

    fn default_model(&self) -> &str {
        &self.config.default_model
    }

    fn available_models(&self) -> Vec<&str> {
        // Popular models available on OpenRouter (based on 2025 rankings)
        vec![
            // Top performers
            "anthropic/claude-3.5-sonnet",
            "anthropic/claude-4-sonnet-20250522",
            "openai/gpt-4o",
            "openai/gpt-4o-mini",
            "openai/gpt-4.1-mini-2025-04-14",
            "google/gemini-2.5-flash",
            "google/gemini-2.0-flash-001",
            "google/gemini-2.5-pro",
            // Popular coding models
            "x-ai/grok-code-fast-1",
            "z-ai/glm-4.5",
            "z-ai/glm-4.5v",
            // Budget-friendly options
            "deepseek/deepseek-chat",
            "qwen/qwen-2.5-72b-instruct",
            "mistralai/mistral-small-3.2-24b-instruct-2506",
            // Legacy but still useful
            "anthropic/claude-3-opus",
            "anthropic/claude-3-haiku",
            "openai/gpt-4-turbo",
            "meta-llama/llama-3.1-405b-instruct",
            "meta-llama/llama-3.1-70b-instruct",
        ]
    }

    async fn generate(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        // Validate model is in our supported list (optional - OpenRouter supports many models)
        if !self.available_models().is_empty()
            && !self.available_models().contains(&request.model.as_str())
        {
            warn!(
                event = "openrouter_model_not_in_list",
                model = %request.model,
                "Model not in predefined list, but continuing with OpenRouter request"
            );
        }

        // Convert images warning for non-vision models
        if !request.images.is_empty() {
            let is_vision_model = request.model.contains("vision")
                || request.model.contains("gpt-4o")
                || request.model.contains("claude-3")
                || request.model.contains("claude-4")
                || request.model.contains("gemini")
                || request.model.contains("glm-4.5v")
                || request.model.contains("grok");

            if !is_vision_model {
                warn!(
                    event = "openrouter_images_not_supported_for_model",
                    model = %request.model,
                    "Images may not be supported for model {} - ignoring {} images",
                    request.model,
                    request.images.len()
                );
            }
        }

        let openrouter_messages: Vec<OpenRouterMessage> = request
            .messages
            .iter()
            .map(|msg| self.convert_message(msg))
            .collect();

        let openrouter_tools = request
            .tools
            .map(|tools| tools.iter().map(|t| self.convert_tool(t)).collect());

        let openrouter_request = OpenRouterRequest {
            model: request.model.clone(),
            messages: openrouter_messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            tools: openrouter_tools,
            tool_choice: request.tool_choice,
            stream: request.stream,
        };

        info!(
            event = "openrouter_api_request",
            model = %request.model,
            message_count = request.messages.len(),
            has_tools = openrouter_request.tools.is_some(),
            "Sending request to OpenRouter API"
        );

        // Debug the full request being sent
        info!(
            event = "openrouter_request_debug",
            full_request = ?openrouter_request,
            "Full request being sent to OpenRouter API"
        );

        let url = format!("{}/chat/completions", self.config.api_base_url);

        // Retry logic with exponential backoff for rate limits and capacity issues
        let max_retries = 3;
        let mut attempt = 0;

        loop {
            let response = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("HTTP-Referer", "https://github.com/your-bot/chloe") // Optional: for OpenRouter analytics
                .header("X-Title", "Chloe Discord Bot") // Optional: for OpenRouter analytics
                .json(&openrouter_request)
                .send()
                .await?;

            let status = response.status();
            if status.is_success() {
                // Success - process the response
                let openrouter_response: OpenRouterResponse = response.json().await?;
                return self.convert_response(openrouter_response);
            }

            let error_text = response.text().await.unwrap_or_default();

            // Check if this is a retryable error
            let is_retryable = status == 429 // Rate limit
                || status == 503 // Service unavailable
                || status == 502 // Bad gateway
                || (status == 500 && error_text.contains("capacity")) // Over capacity
                || error_text.contains("try again");

            if is_retryable && attempt < max_retries {
                attempt += 1;
                let delay_ms = 1000_u64 * (2_u64.pow(attempt as u32 - 1)); // Exponential backoff: 1s, 2s, 4s

                warn!(
                    event = "openrouter_retryable_error",
                    attempt = attempt,
                    max_retries = max_retries,
                    delay_ms = delay_ms,
                    status_code = %status,
                    "Retrying OpenRouter request due to retryable error"
                );

                tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                continue;
            }

            // Non-recoverable error or max retries exceeded
            error!(
                event = "openrouter_api_error",
                status_code = %status,
                error_text = %error_text,
                attempt = attempt,
                "OpenRouter API request failed"
            );

            return Err(match status.as_u16() {
                401 => LlmError::AuthenticationFailed,
                402 => LlmError::ProviderError("Insufficient credits on OpenRouter".to_string()),
                429 => LlmError::RateLimitExceeded,
                400 => {
                    if error_text.contains("model") && error_text.contains("not found") {
                        LlmError::ModelNotAvailable(openrouter_request.model)
                    } else {
                        LlmError::InvalidRequest(error_text)
                    }
                }
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
