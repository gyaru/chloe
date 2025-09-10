use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolName {
    WebSearch,
    Fetch,
    #[serde(rename = "discord_send_message")]
    DiscordSendMessage,
    #[serde(rename = "discord_add_reaction")]
    DiscordAddReaction,
    #[serde(rename = "generate_image")]
    GenerateImage,
    #[serde(rename = "playwright_web_content")]
    PlaywrightWebContent,
    #[serde(rename = "get_time")]
    GetTime,
    Calculator,
}

impl ToolName {
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "web_search" => Ok(Self::WebSearch),
            "fetch" => Ok(Self::Fetch),
            "discord_send_message" => Ok(Self::DiscordSendMessage),
            "discord_add_reaction" => Ok(Self::DiscordAddReaction),
            "generate_image" => Ok(Self::GenerateImage),
            "playwright_web_content" => Ok(Self::PlaywrightWebContent),
            "get_time" => Ok(Self::GetTime),
            "calculator" => Ok(Self::Calculator),
            _ => Err(anyhow!("Unknown tool name: {}", s)),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WebSearch => "web_search",
            Self::Fetch => "fetch",
            Self::DiscordSendMessage => "discord_send_message",
            Self::DiscordAddReaction => "discord_add_reaction",
            Self::GenerateImage => "generate_image",
            Self::PlaywrightWebContent => "playwright_web_content",
            Self::GetTime => "get_time",
            Self::Calculator => "calculator",
        }
    }

    pub fn needs_result_feedback(&self) -> bool {
        match self {
            Self::DiscordSendMessage | Self::DiscordAddReaction => false,
            _ => true,
        }
    }
}

impl fmt::Display for ToolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}