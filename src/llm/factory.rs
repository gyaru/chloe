use crate::llm::types::LlmError;
use crate::llm::{GroqProvider, LlmProvider, OpenRouterProvider, ZaiProvider};
use std::env;
use std::sync::Arc;
use tracing::{info, warn};

pub struct ProviderFactory;

#[derive(Debug, Clone)]
pub enum ProviderType {
    Groq,
    Zai,
    OpenRouter,
}

impl ProviderFactory {
    /// Create a provider based on environment variable or fallback
    pub fn create_provider() -> Result<Arc<dyn LlmProvider>, LlmError> {
        let provider_type = Self::determine_provider_type();

        info!(
            event = "provider_creation",
            provider = ?provider_type,
            "Creating LLM provider"
        );

        match provider_type {
            ProviderType::Groq => {
                let groq = GroqProvider::new()?;
                info!(
                    event = "provider_created",
                    provider = "groq",
                    model = groq.default_model(),
                    supports_tools = groq.supports_tools(),
                    supports_images = groq.supports_images(),
                    model_source = if std::env::var("LLM_MODEL").is_ok() {
                        "LLM_MODEL env var"
                    } else {
                        "provider default"
                    },
                    "Groq provider created successfully"
                );
                Ok(Arc::new(groq))
            }
            ProviderType::Zai => {
                let zai = ZaiProvider::new()?;
                info!(
                    event = "provider_created",
                    provider = "zai",
                    model = zai.default_model(),
                    supports_tools = zai.supports_tools(),
                    supports_images = zai.supports_images(),
                    model_source = if std::env::var("LLM_MODEL").is_ok() {
                        "LLM_MODEL env var"
                    } else {
                        "provider default"
                    },
                    "z.AI provider created successfully"
                );
                Ok(Arc::new(zai))
            }
            ProviderType::OpenRouter => {
                let openrouter = OpenRouterProvider::new()?;
                info!(
                    event = "provider_created",
                    provider = "openrouter",
                    model = openrouter.default_model(),
                    supports_tools = openrouter.supports_tools(),
                    supports_images = openrouter.supports_images(),
                    model_source = if std::env::var("LLM_MODEL").is_ok() {
                        "LLM_MODEL env var"
                    } else {
                        "provider default"
                    },
                    "OpenRouter provider created successfully"
                );
                Ok(Arc::new(openrouter))
            }
        }
    }

    /// Create a specific provider type
    pub fn create_groq_provider() -> Result<Arc<dyn LlmProvider>, LlmError> {
        let groq = GroqProvider::new()?;
        info!(
            event = "groq_provider_created",
            model = groq.default_model(),
            model_source = if std::env::var("LLM_MODEL").is_ok() {
                "LLM_MODEL env var"
            } else {
                "provider default"
            },
            "Groq provider created"
        );
        Ok(Arc::new(groq))
    }

    /// Create z.AI provider
    pub fn create_zai_provider() -> Result<Arc<dyn LlmProvider>, LlmError> {
        let zai = ZaiProvider::new()?;
        info!(
            event = "zai_provider_created",
            model = zai.default_model(),
            model_source = if std::env::var("LLM_MODEL").is_ok() {
                "LLM_MODEL env var"
            } else {
                "provider default"
            },
            "z.AI provider created"
        );
        Ok(Arc::new(zai))
    }

    /// Create OpenRouter provider
    pub fn create_openrouter_provider() -> Result<Arc<dyn LlmProvider>, LlmError> {
        let openrouter = OpenRouterProvider::new()?;
        info!(
            event = "openrouter_provider_created",
            model = openrouter.default_model(),
            model_source = if std::env::var("LLM_MODEL").is_ok() {
                "LLM_MODEL env var"
            } else {
                "provider default"
            },
            "OpenRouter provider created"
        );
        Ok(Arc::new(openrouter))
    }

    /// Determine which provider to use based on environment variables
    fn determine_provider_type() -> ProviderType {
        // Check for explicit provider preference
        if let Ok(provider) = env::var("LLM_PROVIDER") {
            let provider_lower = provider.to_lowercase();
            match provider_lower.as_str() {
                "groq" => {
                    info!(
                        event = "provider_selection",
                        source = "LLM_PROVIDER",
                        selected = "groq",
                        "Provider explicitly set to Groq"
                    );
                    return ProviderType::Groq;
                }
                "zai" | "z.ai" => {
                    info!(
                        event = "provider_selection",
                        source = "LLM_PROVIDER",
                        selected = "zai",
                        "Provider explicitly set to z.AI"
                    );
                    return ProviderType::Zai;
                }
                "openrouter" | "or" => {
                    info!(
                        event = "provider_selection",
                        source = "LLM_PROVIDER",
                        selected = "openrouter",
                        "Provider explicitly set to OpenRouter"
                    );
                    return ProviderType::OpenRouter;
                }
                _ => {
                    warn!(
                        event = "provider_selection_invalid",
                        invalid_provider = %provider,
                        "Invalid LLM_PROVIDER value, falling back to auto-detection"
                    );
                }
            }
        }

        // Auto-detect based on available API keys
        let has_groq_key = env::var("GROQ_API_KEY").is_ok_and(|key| !key.is_empty());
        let has_zai_key = env::var("ZAI_API_KEY").is_ok_and(|key| !key.is_empty());
        let has_openrouter_key = env::var("OPENROUTER_API_KEY").is_ok_and(|key| !key.is_empty());

        // Priority order: OpenRouter > z.AI > Groq
        // OpenRouter has the most model variety, z.AI has better tool calling than Groq
        match (has_openrouter_key, has_zai_key, has_groq_key) {
            (true, _, _) => {
                info!(
                    event = "provider_selection",
                    source = "auto_detect",
                    selected = "openrouter",
                    reason = "openrouter_key_available",
                    "OpenRouter API key available, using OpenRouter"
                );
                ProviderType::OpenRouter
            }
            (false, true, _) => {
                info!(
                    event = "provider_selection",
                    source = "auto_detect",
                    selected = "zai",
                    reason = "zai_key_available",
                    "z.AI API key available, using z.AI"
                );
                ProviderType::Zai
            }
            (false, false, true) => {
                info!(
                    event = "provider_selection",
                    source = "auto_detect",
                    selected = "groq",
                    reason = "only_groq_key_available",
                    "Only Groq API key available"
                );
                ProviderType::Groq
            }
            (false, false, false) => {
                warn!(
                    event = "provider_selection",
                    source = "auto_detect",
                    selected = "groq",
                    reason = "no_keys_available_fallback",
                    "No API keys available, defaulting to Groq (will likely fail)"
                );
                ProviderType::Groq
            }
        }
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderType::Groq => write!(f, "groq"),
            ProviderType::Zai => write!(f, "z.ai"),
            ProviderType::OpenRouter => write!(f, "openrouter"),
        }
    }
}
