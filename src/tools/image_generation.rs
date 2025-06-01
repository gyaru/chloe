use super::Tool;
use serde_json::{json, Value};
use std::collections::HashMap;

pub struct ImageGenerationTool {
    client: reqwest::Client,
    api_key: Option<String>,
}

impl ImageGenerationTool {
    pub fn new() -> Self {
        let api_key = std::env::var("GEMINI_API_KEY").ok();
        
        Self {
            client: reqwest::Client::new(),
            api_key,
        }
    }
}

#[async_trait::async_trait]
impl Tool for ImageGenerationTool {
    fn name(&self) -> &str {
        "generate_image"
    }

    fn description(&self) -> &str {
        "Generate images using Google's Imagen AI. Provide a detailed description of what you want to create. MUST be used when users ask you to create, generate, make, or draw images, pictures, or visual content."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "A detailed description of the image to generate"
                }
            },
            "required": ["prompt"]
        })
    }

    async fn execute(&self, parameters: HashMap<String, Value>, _discord_context: Option<&super::DiscordContext>) -> Result<String, String> {
        let prompt = parameters.get("prompt")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid 'prompt' parameter")?;

        let api_key = self.api_key.as_ref()
            .ok_or("GEMINI_API_KEY environment variable not set")?;

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/imagen-3.0-generate-002:predict?key={}",
            api_key
        );

        let request_body = json!({
            "instances": [{
                    "prompt": prompt
            }],
            "parameters": {
                "sampleCount": 4,
            }
        });

        let response = self.client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Failed to send request to Imagen API: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Imagen API request failed with status {}: {}", status, error_text));
        }

        let response_json: Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse Imagen API response: {}", e))?;

        // Extract the base64 image data from the response
        if let Some(predictions) = response_json.get("predictions").and_then(|p| p.as_array()) {
            if let Some(prediction) = predictions.get(0) {
                if let Some(base64_data) = prediction.get("bytesBase64Encoded").and_then(|d| d.as_str()) {
                    let mime_type = prediction.get("mimeType")
                        .and_then(|m| m.as_str())
                        .unwrap_or("image/png");
                    
                    // Create a data URL for the image
                    let image_url = format!("data:{};base64,{}", mime_type, base64_data);
                    
                    return Ok(format!("{}", image_url));
                }
            }
        }

        Err("Failed to extract image data from Imagen API response".to_string())
    }
}