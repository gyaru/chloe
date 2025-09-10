use thiserror::Error;

#[derive(Error, Debug)]
pub enum BotError {
    #[error("Environment variable '{0}' not set")]
    EnvVar(String),
    
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    
    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),
    
    #[error("HTTP request error: {0}")]
    Http(#[from] reqwest::Error),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    
    #[error("Discord API error: {0}")]
    Discord(String),
    
    #[error("LLM API error: {0}")]
    LlmApi(String),
    
    #[error("Tool execution error: {0}")]
    ToolExecution(String),
    
    #[error("Rate limit exceeded")]
    RateLimit,
    
    #[error("Operation timed out")]
    Timeout,
    
    #[error("Invalid configuration: {0}")]
    Config(String),
    
    #[error("Regex compilation error: {0}")]
    Regex(#[from] regex::Error),
    
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, BotError>;

// Conversion utilities
impl BotError {
    pub fn env_var(var: &str) -> Self {
        Self::EnvVar(var.to_string())
    }
    
    pub fn discord<S: Into<String>>(msg: S) -> Self {
        Self::Discord(msg.into())
    }
    
    pub fn llm_api<S: Into<String>>(msg: S) -> Self {
        Self::LlmApi(msg.into())
    }
    
    pub fn tool<S: Into<String>>(msg: S) -> Self {
        Self::ToolExecution(msg.into())
    }
    
    pub fn config<S: Into<String>>(msg: S) -> Self {
        Self::Config(msg.into())
    }
}