use super::common::make_workspace;
use super::super::prompt::build_system_prompt;

#[test]
fn aieos_identity_from_file() {
    use crate::openhuman::config::IdentityConfig;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let identity_path = tmp.path().join("aieos_identity.json");

    // Write AIEOS identity file
    let aieos_json = r#"{
        "identity": {
            "names": {"first": "Nova", "nickname": "Nov"},
            "bio": "A helpful AI assistant.",
            "origin": "Silicon Valley"
        },
        "psychology": {
            "mbti": "INTJ",
            "moral_compass": ["Be helpful", "Do no harm"]
        },
        "linguistics": {
            "style": "concise",
            "formality": "casual"
        }
    }"#;
    std::fs::write(&identity_path, aieos_json).unwrap();

    // Create identity config pointing to the file
    let config = IdentityConfig {
        format: "aieos".into(),
        aieos_path: Some("aieos_identity.json".into()),
        aieos_inline: None,
    };

    let prompt = build_system_prompt(tmp.path(), "model", &[], &[], Some(&config), None);

    // Should contain AIEOS sections
    assert!(prompt.contains("## Identity"));
    assert!(prompt.contains("**Name:** Nova"));
    assert!(prompt.contains("**Nickname:** Nov"));
    assert!(prompt.contains("**Bio:** A helpful AI assistant."));
    assert!(prompt.contains("**Origin:** Silicon Valley"));

    assert!(prompt.contains("## Personality"));
    assert!(prompt.contains("**MBTI:** INTJ"));
    assert!(prompt.contains("**Moral Compass:**"));
    assert!(prompt.contains("- Be helpful"));

    assert!(prompt.contains("## Communication Style"));
    assert!(prompt.contains("**Style:** concise"));
    assert!(prompt.contains("**Formality Level:** casual"));

    // Should NOT contain OpenClaw bootstrap file headers
    assert!(!prompt.contains("### SOUL.md"));
    assert!(!prompt.contains("### IDENTITY.md"));
    assert!(!prompt.contains("[File not found"));
}

#[test]
fn aieos_identity_from_inline() {
    use crate::openhuman::config::IdentityConfig;

    let config = IdentityConfig {
        format: "aieos".into(),
        aieos_path: None,
        aieos_inline: Some(r#"{"identity":{"names":{"first":"Claw"}}}"#.into()),
    };

    let prompt = build_system_prompt(
        std::env::temp_dir().as_path(),
        "model",
        &[],
        &[],
        Some(&config),
        None,
    );

    assert!(prompt.contains("**Name:** Claw"));
    assert!(prompt.contains("## Identity"));
}

#[test]
fn aieos_fallback_to_openclaw_on_parse_error() {
    use crate::openhuman::config::IdentityConfig;

    let config = IdentityConfig {
        format: "aieos".into(),
        aieos_path: Some("nonexistent.json".into()),
        aieos_inline: None,
    };

    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], Some(&config), None);

    // Should fall back to OpenClaw format when AIEOS file is not found
    // (Error is logged to stderr with filename, not included in prompt)
    assert!(prompt.contains("### SOUL.md"));
}

#[test]
fn aieos_empty_uses_openclaw() {
    use crate::openhuman::config::IdentityConfig;

    // Format is "aieos" but neither path nor inline is set
    let config = IdentityConfig {
        format: "aieos".into(),
        aieos_path: None,
        aieos_inline: None,
    };

    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], Some(&config), None);

    // Should use OpenClaw format (not configured for AIEOS)
    assert!(prompt.contains("### SOUL.md"));
    assert!(prompt.contains("Be helpful"));
}

#[test]
fn openclaw_format_uses_bootstrap_files() {
    use crate::openhuman::config::IdentityConfig;

    let config = IdentityConfig {
        format: "openclaw".into(),
        aieos_path: Some("identity.json".into()),
        aieos_inline: None,
    };

    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], Some(&config), None);

    // Should use OpenClaw format even if aieos_path is set
    assert!(prompt.contains("### SOUL.md"));
    assert!(prompt.contains("Be helpful"));
    assert!(!prompt.contains("## Identity"));
}

#[test]
fn none_identity_config_uses_openclaw() {
    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    assert!(prompt.contains("### SOUL.md"));
    assert!(prompt.contains("Be helpful"));
}
