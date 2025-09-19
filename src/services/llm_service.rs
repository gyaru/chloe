use crate::llm::{LlmMessage, LlmProvider, LlmRequest, LlmRole};
use crate::settings::Settings;
use anyhow::Result;
use std::sync::Arc;
use tracing::{error, info};

pub struct LlmService {
    provider: Arc<dyn LlmProvider>,
    settings: Arc<Settings>,
}

pub struct LlmResponse {
    pub text: String,
}

impl LlmService {
    pub fn new(provider: Arc<dyn LlmProvider>, settings: Arc<Settings>) -> Result<Self> {
        Ok(Self { provider, settings })
    }

    pub async fn generate_response(
        &self,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<LlmResponse> {
        let messages = vec![
            LlmMessage {
                role: LlmRole::System,
                content: system_prompt.to_string(),
                images: None,
            },
            LlmMessage {
                role: LlmRole::User,
                content: user_message.to_string(),
                images: None,
            },
        ];

        let request = LlmRequest {
            messages,
            tools: None,
            model: None, // Use provider default
        };

        info!(
            event = "llm_request",
            provider = self.provider.name(),
            "Sending request to LLM provider"
        );

        match self.provider.generate(&request).await {
            Ok(response) => {
                info!(
                    event = "llm_response_success",
                    provider = self.provider.name(),
                    "Successfully received LLM response"
                );

                Ok(LlmResponse {
                    text: response.content,
                })
            }
            Err(e) => {
                error!(
                    event = "llm_response_failed",
                    provider = self.provider.name(),
                    error = ?e,
                    "Failed to generate LLM response"
                );
                Err(e.into())
            }
        }
    }

    pub fn get_provider_name(&self) -> &str {
        self.provider.name()
    }
}
