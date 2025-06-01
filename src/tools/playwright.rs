use super::Tool;
use serde_json::{json, Value};
use std::collections::HashMap;

pub struct PlaywrightWebContentTool {
    client: reqwest::Client,
    playwright_url: Option<String>,
}

impl PlaywrightWebContentTool {
    pub fn new() -> Self {
        let playwright_url = std::env::var("PLAYWRIGHT_URL").ok();
        let has_url = playwright_url.is_some();
        
        if !has_url {
            eprintln!("Warning: PLAYWRIGHT_URL environment variable not set. Web content fetching will not work.");
        }
        
        Self {
            client: reqwest::Client::new(),
            playwright_url,
        }
    }
}

#[async_trait::async_trait]
impl Tool for PlaywrightWebContentTool {
    fn name(&self) -> &str {
        "fetch_web_content"
    }

    fn description(&self) -> &str {
        "Fetch and analyze the full content of a web page using Playwright. This tool can access dynamic content, JavaScript-rendered pages, and extract text, links, and other elements. Use this when you need to read the actual content of a specific webpage that users mention or link to."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL of the webpage to fetch and analyze"
                },
                "wait_for": {
                    "type": "string", 
                    "description": "Optional CSS selector to wait for before extracting content (useful for dynamic pages)",
                    "default": null
                },
                "extract_links": {
                    "type": "boolean",
                    "description": "Whether to extract and include links from the page",
                    "default": false
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, parameters: HashMap<String, Value>, _discord_context: Option<&super::DiscordContext>) -> Result<String, String> {
        let url = parameters.get("url")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'url' parameter")?;

        let wait_for = parameters.get("wait_for")
            .and_then(|v| v.as_str());

        let extract_links = parameters.get("extract_links")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let playwright_base_url = self.playwright_url.as_ref()
            .ok_or("PLAYWRIGHT_URL environment variable not set")?;

        // First, get a session ID from the Playwright service
        let session_response = self.client
            .get(playwright_base_url)
            .send()
            .await
            .map_err(|e| format!("Failed to get session from Playwright service: {}", e))?;

        if !session_response.status().is_success() {
            return Err(format!("Failed to get session from Playwright service: {}", session_response.status()));
        }

        // Parse the SSE response to extract session ID
        let session_text = session_response.text().await
            .map_err(|e| format!("Failed to read session response: {}", e))?;
        
        // Extract session ID from SSE format: "data: /sse?sessionId=..."
        let session_id = session_text
            .lines()
            .find(|line| line.starts_with("data: /sse?sessionId="))
            .and_then(|line| line.strip_prefix("data: /sse?sessionId="))
            .ok_or("Could not extract sessionId from Playwright service response")?;

        // Now make the actual request with the session ID
        let playwright_url_with_session = format!("{}?sessionId={}", playwright_base_url, session_id);

        // Create request payload for Playwright service
        let mut request_payload = json!({
            "url": url,
            "extract_text": true,
            "extract_links": extract_links
        });

        if let Some(selector) = wait_for {
            request_payload["wait_for_selector"] = json!(selector);
        }

        let response = self.client
            .post(&playwright_url_with_session)
            .header("Content-Type", "application/json")
            .json(&request_payload)
            .send()
            .await
            .map_err(|e| format!("Failed to send request to Playwright service: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Playwright service request failed with status {}: {}", status, error_text));
        }

        let response_json: Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse Playwright service response: {}", e))?;

        // Extract content from response
        let mut result = format!("**Web Content from: {}**\n\n", url);

        if let Some(title) = response_json.get("title").and_then(|t| t.as_str()) {
            result.push_str(&format!("**Title:** {}\n\n", title));
        }

        if let Some(text_content) = response_json.get("text").and_then(|t| t.as_str()) {
            let truncated_content = if text_content.len() > 3000 {
                format!("{}...\n\n[Content truncated - original length: {} characters]", 
                       &text_content[..3000], text_content.len())
            } else {
                text_content.to_string()
            };
            result.push_str(&format!("**Content:**\n{}\n\n", truncated_content));
        }

        if extract_links {
            if let Some(links) = response_json.get("links").and_then(|l| l.as_array()) {
                if !links.is_empty() {
                    result.push_str("**Links found:**\n");
                    for (i, link) in links.iter().take(10).enumerate() {
                        if let (Some(href), Some(text)) = (
                            link.get("href").and_then(|h| h.as_str()),
                            link.get("text").and_then(|t| t.as_str())
                        ) {
                            result.push_str(&format!("{}. [{}]({})\n", i + 1, text, href));
                        }
                    }
                    if links.len() > 10 {
                        result.push_str(&format!("\n... and {} more links\n", links.len() - 10));
                    }
                    result.push('\n');
                }
            }
        }

        Ok(result)
    }
}