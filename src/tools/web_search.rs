use super::Tool;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Debug, Serialize)]
struct ExaSearchRequest {
    query: String,
    #[serde(rename = "numResults")]
    num_results: u32,
    #[serde(rename = "includeDomains")]
    include_domains: Option<Vec<String>>,
    #[serde(rename = "excludeDomains")]
    exclude_domains: Option<Vec<String>>,
    #[serde(rename = "startCrawlDate")]
    start_crawl_date: Option<String>,
    #[serde(rename = "endCrawlDate")]
    end_crawl_date: Option<String>,
    #[serde(rename = "startPublishedDate")]
    start_published_date: Option<String>,
    #[serde(rename = "endPublishedDate")]
    end_published_date: Option<String>,
    #[serde(rename = "useAutoprompt")]
    use_autoprompt: Option<bool>,
    r#type: Option<String>,
    category: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExaSearchResponse {
    results: Vec<ExaResult>,
    #[serde(rename = "autopromptString")]
    autoprompt_string: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExaResult {
    id: String,
    url: String,
    title: String,
    score: Option<f64>,
    #[serde(rename = "publishedDate")]
    published_date: Option<String>,
    author: Option<String>,
    text: Option<String>,
}

pub struct WebSearchTool {
    client: reqwest::Client,
    api_key: Option<String>,
}

impl WebSearchTool {
    pub fn new() -> Self {
        let api_key = std::env::var("EXA_KEY").ok();
        let has_key = api_key.is_some();

        if !has_key {
            eprintln!("Warning: EXA_KEY environment variable not set. Web search will not work.");
        }

        Self {
            client: reqwest::Client::new(),
            api_key,
        }
    }
}

#[async_trait::async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for current information using Exa AI's neural search. Returns relevant results with titles, URLs, authors, published dates, and content previews. MUST be used whenever users ask you to search for, find, or look up anything including: music, videos, news, products, people, places, current events, or any other information that would benefit from web search."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        parameters: HashMap<String, Value>,
        _discord_context: Option<&super::DiscordContext>,
    ) -> Result<String, String> {
        let query = parameters
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'query' parameter")?;

        let api_key = self
            .api_key
            .as_ref()
            .ok_or("EXA_KEY environment variable not set")?;

        let search_request = ExaSearchRequest {
            query: query.to_string(),
            num_results: 5,
            include_domains: None,
            exclude_domains: None,
            start_crawl_date: None,
            end_crawl_date: None,
            start_published_date: None,
            end_published_date: None,
            use_autoprompt: Some(true),
            r#type: Some("keyword".to_string()),
            category: None,
        };

        let response = self
            .client
            .post("https://api.exa.ai/search")
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .header("x-api-key", api_key)
            .json(&search_request)
            .send()
            .await
            .map_err(|e| format!("Failed to send request to Exa API: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!(
                "Exa API request failed with status {}: {}",
                status, error_text
            ));
        }

        let search_response: ExaSearchResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse Exa API response: {}", e))?;

        if search_response.results.is_empty() {
            return Ok(format!("No search results found for query: '{}'", query));
        }

        let mut result_text = format!("Search results for '{}':\n\n", query);

        if let Some(autoprompt) = &search_response.autoprompt_string {
            result_text.push_str(&format!("Refined query: {}\n\n", autoprompt));
        }

        for (i, result) in search_response.results.iter().enumerate() {
            result_text.push_str(&format!("{}. **{}**\n", i + 1, result.title));
            result_text.push_str(&format!("   URL: {}\n", result.url));

            if let Some(author) = &result.author {
                result_text.push_str(&format!("   Author: {}\n", author));
            }

            if let Some(published_date) = &result.published_date {
                result_text.push_str(&format!("   Published: {}\n", published_date));
            }

            if let Some(score) = result.score {
                result_text.push_str(&format!("   Relevance: {:.2}\n", score));
            }

            if let Some(text) = &result.text {
                let snippet = if text.len() > 200 {
                    format!("{}...", &text[..200])
                } else {
                    text.clone()
                };
                result_text.push_str(&format!("   Preview: {}\n", snippet));
            }

            result_text.push('\n');
        }

        Ok(result_text)
    }
}
