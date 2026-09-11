use super::*;
use tempfile::TempDir;

fn test_profile(id: &str) -> AgentProfile {
    AgentProfile {
        id: id.to_string(),
        name: id.to_string(),
        description: String::new(),
        agent_id: "orchestrator".to_string(),
        model_override: None,
        temperature: None,
        system_prompt_suffix: None,
        allowed_tools: None,
        built_in: false,
        avatar_url: None,
        voice_id: None,
        soul_md: None,
        soul_md_path: None,
        composio_integrations: None,
        memory_sources: None,
        include_agent_conversations: true,
        allowed_skills: None,
        allowed_mcp_servers: None,
        memory_dir_suffix: None,
        is_master: false,
        sort_order: None,
        dedicated_memory: false,
        dedicated_workspace: false,
    }
}

#[test]
fn memory_subdir_for_suffix_patterns() {
    assert_eq!(memory_subdir_for_suffix(""), "memory");
    assert_eq!(memory_subdir_for_suffix("-1"), "memory-1");
    assert_eq!(memory_subdir_for_suffix("-2"), "memory-2");
    assert_eq!(memory_subdir_for_suffix("-10"), "memory-10");
}

#[test]
fn memory_tree_subdir_for_suffix_patterns() {
    assert_eq!(memory_tree_subdir_for_suffix(""), "memory_tree");
    assert_eq!(memory_tree_subdir_for_suffix("-1"), "memory_tree-1");
}

#[test]
fn session_raw_subdir_for_suffix_patterns() {
    assert_eq!(session_raw_subdir_for_suffix(""), "session_raw");
    assert_eq!(session_raw_subdir_for_suffix("-1"), "session_raw-1");
}

#[test]
fn resolve_soul_prefers_profile_home_file() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("personalities").join("alice");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("SOUL.md"), "Home identity for Alice").unwrap();

    let mut profile = test_profile("alice");
    // Both inline and soul_md_path present — the profile-home file still wins.
    profile.soul_md = Some("Inline soul".to_string());
    let result = resolve_personality_soul(tmp.path(), &profile);
    assert_eq!(result.as_deref(), Some("Home identity for Alice"));
}

#[test]
fn resolve_soul_skips_home_file_for_invalid_legacy_id() {
    let tmp = TempDir::new().unwrap();
    // A legacy id that fails validate_profile_id (space + uppercase).
    let legacy_id = "Legacy Id";
    let home = tmp.path().join("personalities").join(legacy_id);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("SOUL.md"), "SHOULD BE SKIPPED").unwrap();

    let mut profile = test_profile("placeholder");
    profile.id = legacy_id.to_string();
    profile.soul_md = Some("Legacy inline soul".to_string());
    // Step 1 (profile-home SOUL.md) is skipped for the invalid id; resolution
    // falls through to the inline value.
    let result = resolve_personality_soul(tmp.path(), &profile);
    assert_eq!(result.as_deref(), Some("Legacy inline soul"));
}

#[test]
fn resolve_soul_empty_home_file_falls_through_to_inline() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("personalities").join("alice");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("SOUL.md"), "   \n").unwrap(); // whitespace only

    let mut profile = test_profile("alice");
    profile.soul_md = Some("Inline wins over empty home file".to_string());
    let result = resolve_personality_soul(tmp.path(), &profile);
    assert_eq!(result.as_deref(), Some("Inline wins over empty home file"));
}

#[test]
fn effective_memory_suffix_dedicated_wins_over_numeric() {
    let mut profile = test_profile("alice");
    // The store auto-assigns a numeric suffix to every non-default profile,
    // so `dedicated_memory` must win over it — otherwise the toggle would be
    // dead code and could never route to the `memory-<id>` subtree.
    profile.memory_dir_suffix = Some("-3".to_string());
    profile.dedicated_memory = true;
    assert_eq!(effective_memory_suffix(&profile), "-alice");
}

#[test]
fn effective_memory_suffix_numeric_retained_when_not_dedicated() {
    let mut profile = test_profile("alice");
    // With dedicated_memory off, the persisted legacy numeric suffix is
    // retained so an existing memory directory is never orphaned.
    profile.memory_dir_suffix = Some("-3".to_string());
    profile.dedicated_memory = false;
    assert_eq!(effective_memory_suffix(&profile), "-3");
}

#[test]
fn effective_memory_suffix_invalid_id_dedicated_falls_back_to_numeric() {
    let mut profile = test_profile("placeholder");
    profile.id = "Bad Id".to_string();
    // An invalid id can't mint a `-<id>` directory even with dedicated on, so
    // it falls back to the persisted numeric suffix rather than the shared
    // tree.
    profile.memory_dir_suffix = Some("-2".to_string());
    profile.dedicated_memory = true;
    assert_eq!(effective_memory_suffix(&profile), "-2");
}

#[test]
fn effective_memory_suffix_dedicated_derives_from_id() {
    let mut profile = test_profile("alice");
    profile.memory_dir_suffix = None;
    profile.dedicated_memory = true;
    assert_eq!(effective_memory_suffix(&profile), "-alice");
}

#[test]
fn effective_memory_suffix_shared_default() {
    let mut profile = test_profile("alice");
    profile.memory_dir_suffix = None;
    profile.dedicated_memory = false;
    assert_eq!(effective_memory_suffix(&profile), "");
}

#[test]
fn effective_memory_suffix_invalid_id_falls_back_to_shared() {
    let mut profile = test_profile("placeholder");
    profile.id = "Bad Id".to_string();
    profile.memory_dir_suffix = None;
    profile.dedicated_memory = true;
    // Invalid id cannot mint a directory name — fall back to shared "".
    assert_eq!(effective_memory_suffix(&profile), "");
}

#[test]
fn effective_memory_suffix_empty_string_suffix_is_not_legacy() {
    let mut profile = test_profile("alice");
    // The default profile stores Some("") — treated as "no legacy suffix", so
    // dedicated_memory (if set) still derives, else shared.
    profile.memory_dir_suffix = Some(String::new());
    profile.dedicated_memory = false;
    assert_eq!(effective_memory_suffix(&profile), "");
}

#[test]
fn resolve_soul_inline_fallback() {
    let tmp = TempDir::new().unwrap();
    let mut profile = test_profile("alice");
    profile.soul_md = Some("I am Alice, a friendly assistant.".to_string());
    let result = resolve_personality_soul(tmp.path(), &profile);
    assert_eq!(result.as_deref(), Some("I am Alice, a friendly assistant."));
}

#[test]
fn resolve_soul_file_takes_precedence() {
    let tmp = TempDir::new().unwrap();
    let soul_path = tmp.path().join("souls").join("alice.md");
    std::fs::create_dir_all(soul_path.parent().unwrap()).unwrap();
    std::fs::write(&soul_path, "File-based soul").unwrap();

    let mut profile = test_profile("alice");
    profile.soul_md_path = Some("souls/alice.md".to_string());
    profile.soul_md = Some("Inline soul".to_string());
    let result = resolve_personality_soul(tmp.path(), &profile);
    assert_eq!(result.as_deref(), Some("File-based soul"));
}

#[test]
fn resolve_soul_returns_none_when_empty() {
    let tmp = TempDir::new().unwrap();
    let profile = test_profile("alice");
    let result = resolve_personality_soul(tmp.path(), &profile);
    assert!(result.is_none());
}

#[test]
fn resolve_memory_md_from_personality_dir() {
    let tmp = TempDir::new().unwrap();
    let mem_path = tmp
        .path()
        .join("personalities")
        .join("alice")
        .join("MEMORY.md");
    std::fs::create_dir_all(mem_path.parent().unwrap()).unwrap();
    std::fs::write(&mem_path, "Alice remembers things.").unwrap();

    let profile = test_profile("alice");
    let result = resolve_personality_memory_md(tmp.path(), &profile);
    assert_eq!(result.as_deref(), Some("Alice remembers things."));
}

#[test]
fn resolve_memory_md_returns_none_when_missing() {
    let tmp = TempDir::new().unwrap();
    let profile = test_profile("alice");
    let result = resolve_personality_memory_md(tmp.path(), &profile);
    assert!(result.is_none());
}

#[test]
fn profile_session_signature_tracks_profile_file_edits() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("personalities/alice");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("SOUL.md"), "first soul").unwrap();
    std::fs::write(home.join("MEMORY.md"), "first memory").unwrap();
    let profile = test_profile("alice");

    let original = profile_session_signature(tmp.path(), &profile);
    std::fs::write(home.join("SOUL.md"), "second soul").unwrap();
    let after_soul_edit = profile_session_signature(tmp.path(), &profile);
    assert_ne!(original, after_soul_edit);

    std::fs::write(home.join("MEMORY.md"), "second memory").unwrap();
    let after_memory_edit = profile_session_signature(tmp.path(), &profile);
    assert_ne!(after_soul_edit, after_memory_edit);
}

#[test]
fn personality_context_from_profile() {
    let tmp = TempDir::new().unwrap();
    let mut profile = test_profile("bob");
    profile.memory_dir_suffix = Some("-1".to_string());
    profile.voice_id = Some("voice-xyz".to_string());
    profile.composio_integrations = Some(vec!["slack".to_string()]);
    profile.soul_md = Some("I am Bob.".to_string());

    let ctx = PersonalityContext::from_profile(tmp.path(), profile);
    assert_eq!(ctx.memory_suffix, "-1");
    assert_eq!(ctx.voice_id.as_deref(), Some("voice-xyz"));
    assert_eq!(ctx.soul_md_override.as_deref(), Some("I am Bob."));
    assert_eq!(ctx.composio_allowlist.as_ref().unwrap(), &["slack"]);
}

#[derive(Clone)]
struct FakeIntegration {
    toolkit: String,
}
impl HasToolkit for FakeIntegration {
    fn toolkit_name(&self) -> &str {
        &self.toolkit
    }
}

#[test]
fn filter_integrations_none_passthrough() {
    let all = vec![
        FakeIntegration {
            toolkit: "slack".into(),
        },
        FakeIntegration {
            toolkit: "gmail".into(),
        },
    ];
    let filtered = filter_integrations(&all, None);
    assert_eq!(filtered.len(), 2);
}

#[test]
fn filter_integrations_allowlist() {
    let all = vec![
        FakeIntegration {
            toolkit: "slack".into(),
        },
        FakeIntegration {
            toolkit: "gmail".into(),
        },
        FakeIntegration {
            toolkit: "notion".into(),
        },
    ];
    let allowed = vec!["slack".to_string(), "notion".to_string()];
    let filtered = filter_integrations(&all, Some(&allowed));
    assert_eq!(filtered.len(), 2);
    assert_eq!(filtered[0].toolkit, "slack");
    assert_eq!(filtered[1].toolkit, "notion");
}

#[test]
fn filter_integrations_empty_allowlist() {
    let all = vec![FakeIntegration {
        toolkit: "slack".into(),
    }];
    let allowed: Vec<String> = vec![];
    let filtered = filter_integrations(&all, Some(&allowed));
    assert!(filtered.is_empty());
}

fn connected_integration(toolkit: &str) -> crate::openhuman::agent::prompts::ConnectedIntegration {
    crate::openhuman::agent::prompts::ConnectedIntegration {
        toolkit: toolkit.to_string(),
        description: String::new(),
        tools: Vec::new(),
        gated_tools: Vec::new(),
        connected: true,
        connections: Vec::new(),
        non_active_status: None,
    }
}

#[test]
fn filter_connected_integrations_by_profile_allowlist() {
    // The HasToolkit impl lets the per-profile connector gate reuse
    // filter_integrations on the real ConnectedIntegration type.
    let all = vec![
        connected_integration("gmail"),
        connected_integration("slack"),
        connected_integration("notion"),
    ];
    assert_eq!(filter_integrations(&all, None).len(), 3);
    let allow = vec!["gmail".to_string(), "notion".to_string()];
    let filtered = filter_integrations(&all, Some(&allow));
    let kept: Vec<&str> = filtered.iter().map(|c| c.toolkit_name()).collect();
    assert_eq!(kept, vec!["gmail", "notion"]);
}
