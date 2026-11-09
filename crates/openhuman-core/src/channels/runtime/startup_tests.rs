//! Tests for the chat-workload resolver wired into channel runtime startup.
//!
//! Issue #3098 sub-issue 1: prior to this fix, channel runtime startup
//! always built a cloud-only provider chain and used
//! `config.default_model`, ignoring the per-workload `chat_provider`
//! routing string. These tests pin the resolver behavior so the default
//! (cloud) path is preserved for users who haven't picked a local /
//! BYOK model, and the override path activates for those who have.

use super::{resolve_chat_workload, ChatWorkloadResolution};
use crate::config::Config;

fn config_with_chat_provider(s: Option<&str>) -> Config {
    let mut config = Config::default();
    config.chat_provider = s.map(str::to_string);
    config
}

#[test]
fn chat_provider_unset_blank_or_sentinel_resolves_to_cloud() {
    for value in [None, Some(""), Some("cloud"), Some("openhuman")] {
        let config = config_with_chat_provider(value);
        assert!(
            matches!(
                resolve_chat_workload(&config),
                ChatWorkloadResolution::Cloud
            ),
            "{value:?} must resolve to cloud"
        );
    }
}

#[test]
fn chat_provider_local_and_byok_strings_resolve_to_workload() {
    // (configured string, expected slug). The bare `claude_agent_sdk`
    // sentinel has no colon, so its slug is the full string.
    let cases = [
        ("ollama:llama3.2", "ollama"),
        ("lmstudio:qwen2.5:0.5b", "lmstudio"),
        ("openai:gpt-4o", "openai"),
        ("claude_agent_sdk", "claude_agent_sdk"),
    ];
    for (configured, expected_slug) in cases {
        let config = config_with_chat_provider(Some(configured));
        match resolve_chat_workload(&config) {
            ChatWorkloadResolution::Workload {
                provider_string,
                slug,
            } => {
                assert_eq!(provider_string, configured);
                assert_eq!(slug, expected_slug);
            }
            ChatWorkloadResolution::Cloud => {
                panic!("expected Workload for {configured}, got Cloud")
            }
        }
    }
}
