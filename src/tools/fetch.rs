use super::Tool;
use reqwest;
use serde_json::{Value, json};
use std::collections::HashMap;
use tracing::info;

pub struct FetchTool;

impl FetchTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for FetchTool {
    fn name(&self) -> &str {
        "fetch"
    }

    fn description(&self) -> &str {
        "Fetch content from a URL and return the response. Supports GET requests to retrieve web pages, APIs, and other HTTP resources."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch content from"
                }
            },
            "required": ["url"]
        })
    }

    fn needs_discord_context(&self) -> bool {
        false
    }

    fn needs_result_feedback(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        parameters: HashMap<String, Value>,
        _discord_context: Option<&super::DiscordContext>,
    ) -> Result<String, String> {
        let url = parameters
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'url' parameter")?;

        info!(
            event = "fetch_tool_executing",
            url = %url,
            "Fetching content from URL"
        );

        // Build the request
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (compatible; ChloeBot/1.0)")
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        // Execute the request
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch URL: {}", e))?;

        let status = response.status();
        let headers = response.headers().clone();

        info!(
            event = "fetch_response_received",
            url = %url,
            status = %status,
            "Received response from URL"
        );

        // Get content type
        let content_type = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown");

        // Read the response body
        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        // Format the result
        let result = if status.is_success() {
            if body.len() > 50000 {
                // Truncate very large responses
                format!(
                    "Status: {}\nContent-Type: {}\nContent-Length: {} bytes\n\nContent (truncated to 50KB):\n{}...\n\n[Content truncated. Original size: {} bytes]",
                    status,
                    content_type,
                    body.len(),
                    &body[..50000],
                    body.len()
                )
            } else {
                format!(
                    "Status: {}\nContent-Type: {}\nContent-Length: {} bytes\n\nContent:\n{}",
                    status,
                    content_type,
                    body.len(),
                    body
                )
            }
        } else {
            format!(
                "Error: HTTP {}\nContent-Type: {}\n\nResponse:\n{}",
                status, content_type, body
            )
        };

        Ok(result)
    }
}
