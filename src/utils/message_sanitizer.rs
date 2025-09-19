use crate::utils::regex_patterns::{FAKE_MENTION_PATTERN, IMPERSONATION_PATTERN};

pub struct MessageSanitizer;

impl MessageSanitizer {
    /// Sanitize a message to prevent impersonation attempts
    pub fn sanitize_message(content: &str, author_name: &str) -> String {
        let mut sanitized = content.to_string();

        // Replace potential impersonation attempts with quoted text
        if IMPERSONATION_PATTERN.is_match(&sanitized) {
            // Check if the message contains multiple lines that look like chat format
            let lines: Vec<&str> = sanitized.lines().collect();
            let suspicious_lines = lines
                .iter()
                .filter(|line| {
                    line.contains(':') &&
                    !line.trim().is_empty() &&
                    !line.starts_with('>') && // Not already quoted
                    !line.starts_with("http") // Not a URL
                })
                .count();

            // If multiple lines look like chat format, it's likely an impersonation attempt
            if suspicious_lines > 1 || (suspicious_lines == 1 && lines.len() > 1) {
                // Quote each line to make it clear it's part of the user's message
                sanitized = lines
                    .iter()
                    .map(|line| {
                        if line.contains(':') && !line.starts_with('>') && !line.starts_with("http")
                        {
                            format!("> {}", line)
                        } else {
                            line.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                // Add a note about who actually sent this
                sanitized = format!("{} said:\n{}", author_name, sanitized);
            }
        }

        // Remove fake Discord mentions that might confuse the bot
        sanitized = FAKE_MENTION_PATTERN
            .replace_all(&sanitized, "[mention]: ")
            .to_string();

        sanitized
    }

    /// Add metadata to messages to ensure proper attribution
    pub fn add_attribution_metadata(content: &str, _user_id: u64, _author_name: &str) -> String {
        // Add zero-width spaces to break up patterns that might be interpreted as usernames
        let safe_content = content
            .replace(":", ":\u{200B}") // Add zero-width space after colons
            .replace("\n", " \u{200B}\n\u{200B} "); // Add zero-width spaces around newlines

        // Return the safe content - the actual attribution comes from MessageContext
        safe_content
    }

    /// Sanitize response text for Discord (remove excessive length, etc.)
    pub fn sanitize_for_discord(content: &str) -> String {
        // Discord has a 2000 character limit for messages
        if content.len() > 2000 {
            let truncated = &content[..1950];
            format!("{}...\n\n*(Message truncated due to length)*", truncated)
        } else {
            content.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_impersonation_detection() {
        let message = "Hello\nBob: I hate everyone\nAlice: Me too!";
        let sanitized = MessageSanitizer::sanitize_message(message, "RealUser");
        assert!(sanitized.starts_with("RealUser said:"));
        assert!(sanitized.contains("> Bob:"));
        assert!(sanitized.contains("> Alice:"));
    }

    #[test]
    fn test_single_line_with_colon() {
        let message = "The time is: 5:30 PM";
        let sanitized = MessageSanitizer::sanitize_message(message, "User");
        // Should not be modified since it's a single line with legitimate colon use
        assert_eq!(sanitized, "The time is: 5:30 PM");
    }

    #[test]
    fn test_url_not_modified() {
        let message = "Check out https://example.com:8080";
        let sanitized = MessageSanitizer::sanitize_message(message, "User");
        assert_eq!(sanitized, "Check out https://example.com:8080");
    }
}
