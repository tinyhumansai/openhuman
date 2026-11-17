use super::*;
use crate::config::schema::voice_live::{LIVE_PROVIDER_GEMINI_HOSTED, LIVE_PROVIDER_SARVAM};

fn message(sender: &str, content: &str) -> ConversationMessage {
    ConversationMessage {
        id: format!("{sender}-{content}"),
        content: content.into(),
        message_type: "text".into(),
        extra_metadata: serde_json::Value::Null,
        sender: sender.into(),
        created_at: String::new(),
    }
}

#[test]
fn resolves_the_requested_or_default_provider() {
    let mut config = Config::default();
    assert_eq!(
        resolve_provider(&config, None).unwrap(),
        LIVE_PROVIDER_GEMINI_HOSTED
    );
    assert_eq!(
        resolve_provider(&config, Some(" ")).unwrap(),
        LIVE_PROVIDER_GEMINI_HOSTED
    );
    assert_eq!(
        resolve_provider(&config, Some(LIVE_PROVIDER_SARVAM)).unwrap(),
        LIVE_PROVIDER_SARVAM
    );
    assert_eq!(
        resolve_provider(&config, Some("nope")).unwrap_err().code,
        "invalid_request"
    );
    config.voice_live.default_provider = "broken".into();
    assert!(resolve_provider(&config, None).is_err());
}

#[test]
fn the_prompt_carries_voice_guidance_time_and_recent_messages() {
    let bare = system_prompt(&[]);
    assert!(bare.contains("You are Tiny"));
    assert!(bare.contains("local date and time now"));
    assert!(!bare.contains("conversation so far"));

    let mut recent: Vec<_> = (0..20)
        .map(|i| message(if i % 2 == 0 { "user" } else { "agent" }, &format!("m{i}")))
        .collect();
    recent.push(message("user", "   "));
    recent.push(message("agent", &"x".repeat(CONTEXT_CHARS + 50)));
    let prompt = system_prompt(&recent);
    assert!(prompt.contains("The conversation so far"));
    assert!(!prompt.contains("m0\n"), "older messages are dropped");
    assert!(prompt.contains("User: m10"));
    assert!(prompt.contains("You: m19"));
    let longest = prompt.lines().map(str::len).max().unwrap();
    assert!(longest <= CONTEXT_CHARS + 10);
}
