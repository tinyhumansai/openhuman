use super::*;
use serde_json::json;

#[test]
fn profile_signature_includes_id() {
    let profile = AgentProfile {
        id: "planner".into(),
        name: "Planner".into(),
        description: String::new(),
        agent_id: "planner".into(),
        model_override: None,
        temperature: None,
        system_prompt_suffix: None,
        allowed_tools: None,
        built_in: true,
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
    };
    assert!(profile_signature(&profile).contains("\"planner\""));
}

#[test]
fn backwards_compat_deserialize_without_new_fields() {
    // Pre-profiles-feature payload: none of the new allowlist fields, no
    // include_agent_conversations. Must deserialize with safe defaults.
    let json = json!({
        "activeProfileId": "default",
        "profiles": [{
            "id": "default",
            "name": "Default",
            "description": "The standard OpenHuman orchestrator.",
            "agentId": "orchestrator",
            "builtIn": true
        }]
    });
    let state: AgentProfilesState = serde_json::from_value(json).expect("deserialize");
    let profile = &state.profiles[0];
    assert_eq!(profile.avatar_url, None);
    assert_eq!(profile.voice_id, None);
    assert_eq!(profile.memory_dir_suffix, None);
    assert_eq!(profile.memory_sources, None);
    assert_eq!(profile.allowed_skills, None);
    assert_eq!(profile.allowed_mcp_servers, None);
    // Defaults to true so existing users keep cross-chat recall.
    assert!(profile.include_agent_conversations);
    assert!(!profile.is_master);
    // New home fields default to false so legacy payloads keep behaving.
    assert!(!profile.dedicated_memory);
    assert!(!profile.dedicated_workspace);
}

#[test]
fn new_home_fields_roundtrip_over_camelcase() {
    let json = json!({
        "id": "alice",
        "name": "Alice",
        "description": "",
        "agentId": "orchestrator",
        "builtIn": false,
        "dedicatedMemory": true,
        "dedicatedWorkspace": true
    });
    let profile: AgentProfile = serde_json::from_value(json).expect("deserialize");
    assert!(profile.dedicated_memory);
    assert!(profile.dedicated_workspace);
    // Serializes back out as camelCase.
    let out = serde_json::to_value(&profile).expect("serialize");
    assert_eq!(out["dedicatedMemory"], serde_json::json!(true));
    assert_eq!(out["dedicatedWorkspace"], serde_json::json!(true));
}
