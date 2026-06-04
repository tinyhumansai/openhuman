//! Tests for the chat-workload resolver wired into channel runtime startup.
//!
//! Issue #3098 sub-issue 1: prior to this fix, channel runtime startup
//! always built a cloud-only provider chain and used
//! `config.default_model`, ignoring the per-workload `chat_provider`
//! routing string. These tests pin the resolver behavior so the default
//! (cloud) path is preserved for users who haven't picked a local /
//! BYOK model, and the override path activates for those who have.

use super::{resolve_chat_workload, ChatWorkloadResolution};
use crate::openhuman::config::Config;

fn config_with_chat_provider(s: Option<&str>) -> Config {
    let mut config = Config::default();
    config.chat_provider = s.map(str::to_string);
    config
}

#[test]
fn chat_provider_unset_resolves_to_cloud() {
    let config = config_with_chat_provider(None);
    assert!(matches!(
        resolve_chat_workload(&config),
        ChatWorkloadResolution::Cloud
    ));
}

#[test]
fn chat_provider_blank_resolves_to_cloud() {
    let config = config_with_chat_provider(Some(""));
    assert!(matches!(
        resolve_chat_workload(&config),
        ChatWorkloadResolution::Cloud
    ));
}

#[test]
fn chat_provider_cloud_sentinel_resolves_to_cloud() {
    let config = config_with_chat_provider(Some("cloud"));
    assert!(matches!(
        resolve_chat_workload(&config),
        ChatWorkloadResolution::Cloud
    ));
}

#[test]
fn chat_provider_openhuman_sentinel_resolves_to_cloud() {
    let config = config_with_chat_provider(Some("openhuman"));
    assert!(matches!(
        resolve_chat_workload(&config),
        ChatWorkloadResolution::Cloud
    ));
}

#[test]
fn chat_provider_ollama_resolves_to_workload() {
    let config = config_with_chat_provider(Some("ollama:llama3.2"));
    match resolve_chat_workload(&config) {
        ChatWorkloadResolution::Workload {
            provider_string,
            slug,
        } => {
            assert_eq!(provider_string, "ollama:llama3.2");
            assert_eq!(slug, "ollama");
        }
        ChatWorkloadResolution::Cloud => panic!("expected Workload for ollama, got Cloud"),
    }
}

#[test]
fn chat_provider_lmstudio_resolves_to_workload() {
    let config = config_with_chat_provider(Some("lmstudio:qwen2.5:0.5b"));
    match resolve_chat_workload(&config) {
        ChatWorkloadResolution::Workload {
            provider_string,
            slug,
        } => {
            assert_eq!(provider_string, "lmstudio:qwen2.5:0.5b");
            assert_eq!(slug, "lmstudio");
        }
        ChatWorkloadResolution::Cloud => panic!("expected Workload for lmstudio"),
    }
}

#[test]
fn chat_provider_byok_slug_resolves_to_workload() {
    let config = config_with_chat_provider(Some("openai:gpt-4o"));
    match resolve_chat_workload(&config) {
        ChatWorkloadResolution::Workload {
            provider_string,
            slug,
        } => {
            assert_eq!(provider_string, "openai:gpt-4o");
            assert_eq!(slug, "openai");
        }
        ChatWorkloadResolution::Cloud => panic!("expected Workload for byok slug"),
    }
}

#[test]
fn chat_provider_claude_agent_sdk_resolves_to_workload() {
    // Bare sentinel (no colon) — slug is the full string.
    let config = config_with_chat_provider(Some("claude_agent_sdk"));
    match resolve_chat_workload(&config) {
        ChatWorkloadResolution::Workload {
            provider_string,
            slug,
        } => {
            assert_eq!(provider_string, "claude_agent_sdk");
            assert_eq!(slug, "claude_agent_sdk");
        }
        ChatWorkloadResolution::Cloud => panic!("expected Workload for claude_agent_sdk"),
    }
}
