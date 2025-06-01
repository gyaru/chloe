use super::Tool;
use serde_json::{json, Value};
use std::collections::HashMap;
use chrono::{DateTime, Utc};

pub struct GetTimeTool;

#[async_trait::async_trait]
impl Tool for GetTimeTool {
    fn name(&self) -> &str {
        "get_current_time"
    }

    fn description(&self) -> &str {
        "Get the current date and time in UTC"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(&self, _parameters: HashMap<String, Value>, _discord_context: Option<&super::DiscordContext>) -> Result<String, String> {
        let now: DateTime<Utc> = Utc::now();
        Ok(format!("Current UTC time: {}", now.format("%Y-%m-%d %H:%M:%S UTC")))
    }
}