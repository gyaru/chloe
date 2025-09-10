use once_cell::sync::Lazy;
use regex::Regex;
use tracing::error;

// URL matching pattern
pub static URL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://[^\s<>]+")
        .unwrap_or_else(|e| {
            error!("Failed to compile URL_REGEX: {}", e);
            // Fallback to a very simple pattern that matches nothing
            Regex::new(r"^$").unwrap()
        })
});

// Image URL pattern
pub static IMAGE_URL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:https?://[^\s<>]+\.(?:jpg|jpeg|png|gif|webp|bmp)(?:\?[^\s<>]*)?|data:image/[^;]+;base64,[A-Za-z0-9+/=]+)")
        .unwrap_or_else(|e| {
            error!("Failed to compile IMAGE_URL_REGEX: {}", e);
            Regex::new(r"^$").unwrap()
        })
});

// Discord mention pattern
pub static MENTION_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"<[@#&!]?\d+>")
        .unwrap_or_else(|e| {
            error!("Failed to compile MENTION_REGEX: {}", e);
            Regex::new(r"^$").unwrap()
        })
});

// Emoticon pattern
pub static EMOTICON_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\([⊙ಠ]_[⊙ಠ]\)|[ʅ][^ʃ]*[ʃ]|[วงէ]\s*[（(][^)）]*[▿][^)）]*[)）]\s*[วงէ]|[（(][^)）]*[`´′''‛‚ωдノヽ･ｰー〜～∀○●◯﹏‿⌒▽ಠㅁㅂㅠㅜㅡ_\-\^><°º¬¯\\\/TvVuU・·*Д⊙][^)）]*[)）]|（[^（）]*[`´′''‛‚ωдノヽ･ｰー〜～∀○●◯﹏‿⌒▽ಠㅁㅂㅠㅜㅡ_\-\^><°º¬¯\\\/TvVuU・·*Д⊙][^（）]*）|ヽ\([^)]*\)ノ")
        .unwrap_or_else(|e| {
            error!("Failed to compile EMOTICON_REGEX: {}", e);
            Regex::new(r"^$").unwrap()
        })
});

// Escaped markdown character pattern
pub static ESCAPED_CHAR_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\\[*_`~|>]")
        .unwrap_or_else(|e| {
            error!("Failed to compile ESCAPED_CHAR_REGEX: {}", e);
            Regex::new(r"^$").unwrap()
        })
});

// Impersonation pattern
pub static IMPERSONATION_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^([A-Za-z0-9_\-\.]+\s*:\s*.+)$")
        .unwrap_or_else(|e| {
            error!("Failed to compile IMPERSONATION_PATTERN: {}", e);
            Regex::new(r"^$").unwrap()
        })
});

// Fake mention pattern
pub static FAKE_MENTION_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"<@!?\d+>\s*:\s*")
        .unwrap_or_else(|e| {
            error!("Failed to compile FAKE_MENTION_PATTERN: {}", e);
            Regex::new(r"^$").unwrap()
        })
});

// Emoji reaction pattern
pub static REACTION_EMOJI_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^<:([a-zA-Z0-9_]+):(\d+)>$")
        .unwrap_or_else(|e| {
            error!("Failed to compile REACTION_EMOJI_REGEX: {}", e);
            Regex::new(r"^$").unwrap()
        })
});

// Guild emoji pattern
pub static GUILD_EMOJI_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^:([a-zA-Z0-9_]+):$")
        .unwrap_or_else(|e| {
            error!("Failed to compile GUILD_EMOJI_REGEX: {}", e);
            Regex::new(r"^$").unwrap()
        })
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_regex() {
        assert!(URL_REGEX.is_match("https://example.com"));
        assert!(URL_REGEX.is_match("http://example.com:8080/path"));
        assert!(!URL_REGEX.is_match("not a url"));
    }

    #[test]
    fn test_mention_regex() {
        assert!(MENTION_REGEX.is_match("<@123456789>"));
        assert!(MENTION_REGEX.is_match("<@!123456789>"));
        assert!(MENTION_REGEX.is_match("<#123456789>"));
        assert!(!MENTION_REGEX.is_match("@username"));
    }
}