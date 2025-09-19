use serde::{Deserialize, Serialize};
use serde_json::Value;

// Request structures
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiRequest {
    pub contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_settings: Option<Vec<SafetySetting>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Content {
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Part {
    Text {
        text: String,
    },
    InlineData {
        inline_data: InlineData,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: FunctionCall,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: FunctionResponse,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineData {
    pub mime_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub args: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionResponse {
    pub name: String,
    pub response: FunctionResponseData,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionResponseData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub function_declarations: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SafetySetting {
    pub category: String,
    pub threshold: String,
}

// Response structures
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiResponse {
    pub candidates: Option<Vec<Candidate>>,
    pub prompt_feedback: Option<PromptFeedback>,
    pub model_version: Option<String>,
    pub response_id: Option<String>,
    pub usage_metadata: Option<UsageMetadata>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub content: Option<ResponseContent>,
    pub finish_reason: Option<String>,
    pub index: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseContent {
    pub parts: Option<Vec<ResponsePart>>,
    pub role: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ResponsePart {
    Text {
        text: String,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: FunctionCall,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptFeedback {
    pub block_reason: Option<String>,
    pub safety_ratings: Option<Vec<SafetyRating>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetyRating {
    pub category: String,
    pub probability: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetadata {
    pub candidates_token_count: Option<i32>,
    pub prompt_token_count: Option<i32>,
    pub total_token_count: Option<i32>,
}

// Builder for GeminiRequest
impl GeminiRequest {
    pub fn new(prompt: &str) -> Self {
        Self {
            contents: vec![Content {
                parts: vec![Part::Text {
                    text: prompt.to_string(),
                }],
            }],
            tools: None,
            safety_settings: None,
        }
    }

    pub fn with_images(mut self, images: &[crate::llm::ImageData]) -> Self {
        if let Some(content) = self.contents.get_mut(0) {
            for image in images {
                content.parts.push(Part::InlineData {
                    inline_data: InlineData {
                        mime_type: image.mime_type.clone(),
                        data: image.base64_data.clone(),
                    },
                });
            }
        }
        self
    }

    pub fn with_tools(mut self, tool_definitions: Vec<Value>) -> Self {
        if !tool_definitions.is_empty() {
            self.tools = Some(vec![Tool {
                function_declarations: tool_definitions,
            }]);
        }
        self
    }

    pub fn with_safety_settings(mut self, settings: Vec<SafetySetting>) -> Self {
        self.safety_settings = Some(settings);
        self
    }

    pub fn add_function_call_parts(
        mut self,
        function_call: &FunctionCall,
        function_response: FunctionResponse,
    ) -> Self {
        if let Some(content) = self.contents.get_mut(0) {
            content.parts.push(Part::FunctionCall {
                function_call: function_call.clone(),
            });
            content
                .parts
                .push(Part::FunctionResponse { function_response });
        }
        self
    }
}

// Helper methods for response parsing
impl GeminiResponse {
    pub fn get_text(&self) -> Option<&str> {
        self.candidates
            .as_ref()?
            .get(0)?
            .content
            .as_ref()?
            .parts
            .as_ref()?
            .iter()
            .find_map(|part| {
                if let ResponsePart::Text { text } = part {
                    Some(text.as_str())
                } else {
                    None
                }
            })
    }

    pub fn get_function_call(&self) -> Option<&FunctionCall> {
        self.candidates
            .as_ref()?
            .get(0)?
            .content
            .as_ref()?
            .parts
            .as_ref()?
            .iter()
            .find_map(|part| {
                if let ResponsePart::FunctionCall { function_call } = part {
                    Some(function_call)
                } else {
                    None
                }
            })
    }

    pub fn has_function_call(&self) -> bool {
        self.get_function_call().is_some()
    }

    pub fn has_text(&self) -> bool {
        self.get_text().is_some()
    }

    pub fn is_blocked(&self) -> bool {
        self.prompt_feedback
            .as_ref()
            .and_then(|f| f.block_reason.as_ref())
            .is_some()
    }

    pub fn get_block_reason(&self) -> Option<&str> {
        self.prompt_feedback
            .as_ref()
            .and_then(|f| f.block_reason.as_deref())
    }
}

// Standard safety settings
pub fn default_safety_settings() -> Vec<SafetySetting> {
    vec![
        SafetySetting {
            category: "HARM_CATEGORY_HARASSMENT".to_string(),
            threshold: "BLOCK_NONE".to_string(),
        },
        SafetySetting {
            category: "HARM_CATEGORY_HATE_SPEECH".to_string(),
            threshold: "BLOCK_MEDIUM_AND_ABOVE".to_string(),
        },
        SafetySetting {
            category: "HARM_CATEGORY_SEXUALLY_EXPLICIT".to_string(),
            threshold: "BLOCK_NONE".to_string(),
        },
        SafetySetting {
            category: "HARM_CATEGORY_DANGEROUS_CONTENT".to_string(),
            threshold: "BLOCK_NONE".to_string(),
        },
    ]
}
