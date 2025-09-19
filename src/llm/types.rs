use serde::{Deserialize, Serialize};
use serde_json::Value;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("JSON parsing error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Authentication failed")]
    AuthenticationFailed,

    #[error("Model not available: {0}")]
    ModelNotAvailable(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Content filtered by safety settings")]
    ContentFiltered,

    #[error("Provider error: {0}")]
    ProviderError(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LlmRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: LlmRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<LlmToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolCall {
    pub id: String,
    pub r#type: String, // Usually "function"
    pub function: LlmFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmFunction {
    pub name: String,
    pub arguments: String, // JSON string
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmTool {
    pub r#type: String, // Usually "function"
    pub function: LlmToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value, // JSON schema
}

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub messages: Vec<LlmMessage>,
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub tools: Option<Vec<LlmTool>>,
    pub tool_choice: Option<String>,
    pub stream: bool,
    pub images: Vec<ImageData>,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<LlmToolCall>>,
    pub finish_reason: Option<String>,
    pub usage: Option<LlmUsage>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmUsage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ImageData {
    pub base64_data: String,
    pub mime_type: String,
}

#[derive(Debug, Clone)]
pub struct LlmToolResponse {
    pub tool_call_id: String,
    pub content: String,
}

// Helper implementations
impl LlmMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: LlmRole::System,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: LlmRole::User,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: LlmRole::Assistant,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant_with_tools(content: impl Into<String>, tool_calls: Vec<LlmToolCall>) -> Self {
        Self {
            role: LlmRole::Assistant,
            content: content.into(),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn tool_response(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: LlmRole::Tool,
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: None,
        }
    }
}

impl LlmRequest {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            messages: Vec::new(),
            model: model.into(),
            temperature: None,
            max_tokens: None,
            tools: None,
            tool_choice: None,
            stream: false,
            images: Vec::new(),
        }
    }

    pub fn with_message(mut self, message: LlmMessage) -> Self {
        self.messages.push(message);
        self
    }

    pub fn with_messages(mut self, messages: Vec<LlmMessage>) -> Self {
        self.messages.extend(messages);
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn with_tools(mut self, tools: Vec<LlmTool>) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn with_images(mut self, images: Vec<ImageData>) -> Self {
        self.images = images;
        self
    }

    pub fn with_stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }
}

// Convert from tool executor's format to LLM format
impl From<Value> for LlmTool {
    fn from(tool_def: Value) -> Self {
        let name = tool_def
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unknown")
            .to_string();

        let description = tool_def
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_string();

        let parameters = tool_def
            .get("parameters")
            .cloned()
            .unwrap_or(Value::Object(serde_json::Map::new()));

        Self {
            r#type: "function".to_string(),
            function: LlmToolFunction {
                name,
                description,
                parameters,
            },
        }
    }
}
