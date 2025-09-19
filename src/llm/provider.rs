use crate::llm::types::{LlmError, LlmRequest, LlmResponse};
use async_trait::async_trait;
use serde_json::Value;

/// Generic trait for LLM providers
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Get the name of this provider
    fn name(&self) -> &str;

    /// Check if this provider supports tool calling
    fn supports_tools(&self) -> bool;

    /// Check if this provider supports image inputs
    fn supports_images(&self) -> bool;

    /// Get the default model for this provider
    fn default_model(&self) -> &str;

    /// Get available models for this provider
    fn available_models(&self) -> Vec<&str>;

    /// Generate a text response from the LLM
    async fn generate(&self, request: LlmRequest) -> Result<LlmResponse, LlmError>;

    /// Validate that a model is available for this provider
    fn validate_model(&self, model: &str) -> Result<(), LlmError> {
        if self.available_models().contains(&model) {
            Ok(())
        } else {
            Err(LlmError::ModelNotAvailable(model.to_string()))
        }
    }

    /// Estimate token count for a given text (rough estimation)
    fn estimate_tokens(&self, text: &str) -> u32 {
        // Rough estimation: ~4 characters per token for most models
        (text.len() as f32 / 4.0).ceil() as u32
    }

    /// Convert tool definitions from the tool executor format to provider format
    fn convert_tools(&self, tool_definitions: Vec<Value>) -> Vec<Value> {
        // Default implementation - providers can override for custom formats
        tool_definitions
    }

    /// Get provider-specific configuration
    fn get_config(&self) -> ProviderConfig;
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub name: String,
    pub api_base_url: String,
    pub default_model: String,
    pub supports_tools: bool,
    pub supports_images: bool,

    pub max_tokens_default: u32,
    pub temperature_default: f32,
}

impl ProviderConfig {
    pub fn new(name: impl Into<String>, api_base_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            api_base_url: api_base_url.into(),
            default_model: "default".to_string(),
            supports_tools: false,
            supports_images: false,

            max_tokens_default: 4096,
            temperature_default: 0.6,
        }
    }

    pub fn with_default_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    pub fn with_tools_support(mut self, supports: bool) -> Self {
        self.supports_tools = supports;
        self
    }

    pub fn with_images_support(mut self, supports: bool) -> Self {
        self.supports_images = supports;
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens_default = max_tokens;
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature_default = temperature;
        self
    }
}
