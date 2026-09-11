use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};
use serde_json::json;
use tinymemory_api::types::MemoryEntry;

fn test_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy::default())
}

/// Read a pinned preference back through the guard — the door the tool
/// wrote it through.
///
/// The tool holds no handle: it resolves the *bound* driver per call, so a
/// fixture store built here would be a different store entirely and could
/// never see the write. That matters most for the absence assertions —
/// against a store the tool never writes to they hold whether or not the
/// refusal under test worked, which is worse than no assertion at all.
async fn stored(key: &str) -> Option<MemoryEntry> {
    active_memory_guard()
        .await
        .expect("a bound memory guard")
        .get(PINNED_PREFERENCES_NAMESPACE, key)
        .await
        .expect("read back through the guard")
}

/// How many rows the guard holds under `key`.
///
/// Filtered by exact key rather than counting the namespace: the pinned
/// namespace is a fixed constant the tool derives, so every test in the
/// process writes into the same one and a namespace-wide count would be a
/// function of what else ran.
async fn stored_row_count(key: &str) -> usize {
    active_memory_guard()
        .await
        .expect("a bound memory guard")
        .list(Some(PINNED_PREFERENCES_NAMESPACE), None, None)
        .await
        .expect("list through the guard")
        .into_iter()
        .filter(|entry| entry.key == key)
        .count()
}

// ── FacetClass ─────────────────────────────────────────────────────────

#[test]
fn facet_class_parse_case_insensitive() {
    assert_eq!(FacetClass::parse("Style"), Some(FacetClass::Style));
    assert_eq!(FacetClass::parse("IDENTITY"), Some(FacetClass::Identity));
    assert_eq!(FacetClass::parse("tooling"), Some(FacetClass::Tooling));
    assert_eq!(FacetClass::parse("veto"), Some(FacetClass::Veto));
    assert_eq!(FacetClass::parse("goal"), Some(FacetClass::Goal));
    assert_eq!(FacetClass::parse("channel"), Some(FacetClass::Channel));
    assert_eq!(FacetClass::parse("unknown"), None);
    assert_eq!(FacetClass::parse(""), None);
}

#[test]
fn facet_class_as_str_round_trips() {
    for class in [
        FacetClass::Style,
        FacetClass::Identity,
        FacetClass::Tooling,
        FacetClass::Veto,
        FacetClass::Goal,
        FacetClass::Channel,
    ] {
        let parsed = FacetClass::parse(class.as_str()).expect("round-trip must succeed");
        assert_eq!(parsed, class);
    }
}

// ── Key / content helpers ───────────────────────────────────────────────

#[test]
fn pinned_key_format() {
    assert_eq!(
        pinned_key(FacetClass::Tooling, "package_manager"),
        "pinned/tooling/package_manager"
    );
    assert_eq!(
        pinned_key(FacetClass::Style, "verbosity"),
        "pinned/style/verbosity"
    );
}

#[test]
fn pinned_content_format() {
    assert_eq!(
        pinned_content(FacetClass::Tooling, "package_manager", "pnpm"),
        "[pinned] (class=tooling) package_manager: pnpm"
    );
}

// ── Tool metadata ───────────────────────────────────────────────────────

#[test]
fn tool_name_and_permission() {
    let tool = RememberPreferenceTool::new(test_security());
    assert_eq!(tool.name(), "remember_preference");
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
}

#[test]
fn schema_has_required_fields() {
    let tool = RememberPreferenceTool::new(test_security());
    let schema = tool.parameters_schema();
    assert_eq!(schema["type"], "object");
    let required = schema["required"].as_array().unwrap();
    let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"class"));
    assert!(names.contains(&"key"));
    assert!(names.contains(&"value"));
}

// ── Argument validation ─────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn missing_class_returns_error() {
    let tool = RememberPreferenceTool::new(test_security());
    let result = tool
        .execute(json!({"key": "timezone", "value": "IST"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("class"));
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn invalid_class_returns_error() {
    let tool = RememberPreferenceTool::new(test_security());
    let result = tool
        .execute(json!({"class": "bogus", "key": "timezone", "value": "IST"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("invalid class"));
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn missing_key_returns_error() {
    let tool = RememberPreferenceTool::new(test_security());
    let result = tool
        .execute(json!({"class": "style", "value": "terse"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("key"));
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn empty_key_returns_error() {
    let tool = RememberPreferenceTool::new(test_security());
    let result = tool
        .execute(json!({"class": "style", "key": "   ", "value": "terse"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("key cannot be empty"));
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn key_with_spaces_returns_error() {
    let tool = RememberPreferenceTool::new(test_security());
    let result = tool
        .execute(json!({"class": "style", "key": "my pref", "value": "terse"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("invalid characters"));
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn missing_value_returns_error() {
    let tool = RememberPreferenceTool::new(test_security());
    let result = tool
        .execute(json!({"class": "tooling", "key": "pkg_mgr"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("value"));
}

// ── Successful upsert ───────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn stores_preference_in_user_profile_namespace() {
    let tool = RememberPreferenceTool::new(test_security());
    let result = tool
        .execute(json!({"class": "tooling", "key": "package_manager", "value": "pnpm"}))
        .await
        .unwrap();
    assert!(!result.is_error, "unexpected error: {}", result.output());
    assert!(result.output().contains("package_manager"));

    let entry = stored("pinned/tooling/package_manager").await;
    assert!(entry.is_some(), "entry must have been stored");
    let entry = entry.unwrap();
    assert_eq!(
        entry.content,
        "[pinned] (class=tooling) package_manager: pnpm"
    );
    assert_eq!(entry.category, MemoryCategory::Core);
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn idempotent_overwrite_does_not_create_duplicate() {
    let tool = RememberPreferenceTool::new(test_security());

    // First write.
    tool.execute(json!({"class": "style", "key": "verbosity", "value": "verbose"}))
        .await
        .unwrap();

    // Overwrite with new value.
    let result = tool
        .execute(json!({"class": "style", "key": "verbosity", "value": "terse"}))
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "overwrite must succeed: {}",
        result.output()
    );

    // Verify the overwritten content via get() which reads the actual content column.
    let entry = stored("pinned/style/verbosity")
        .await
        .expect("entry must exist after overwrite");
    assert_eq!(
        entry.content, "[pinned] (class=style) verbosity: terse",
        "overwritten content must reflect the latest value"
    );

    // Verify no duplicate entries exist via list().
    assert_eq!(
        stored_row_count("pinned/style/verbosity").await,
        1,
        "must not duplicate entries"
    );
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn stores_all_six_classes() {
    let tool = RememberPreferenceTool::new(test_security());

    for (class, key, value) in [
        ("style", "tone", "formal"),
        ("identity", "name", "Alice"),
        ("tooling", "editor", "neovim"),
        ("veto", "no_emoji", "true"),
        ("goal", "ship_feature", "memory refactor"),
        ("channel", "preferred", "slack"),
    ] {
        let result = tool
            .execute(json!({"class": class, "key": key, "value": value}))
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "class={class} failed: {}",
            result.output()
        );
    }

    // Asserted key by key rather than as a namespace count: the pinned
    // namespace is a fixed constant, so every test in the process writes
    // into the same one and a total would be a function of what else ran.
    for key in [
        "pinned/style/tone",
        "pinned/identity/name",
        "pinned/tooling/editor",
        "pinned/veto/no_emoji",
        "pinned/goal/ship_feature",
        "pinned/channel/preferred",
    ] {
        assert!(stored(key).await.is_some(), "{key} must have been stored");
    }
}

// ── Security gate ───────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn blocked_in_readonly_mode() {
    let readonly = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let tool = RememberPreferenceTool::new(readonly);
    // A key no other test writes: the absence assertion now runs against
    // the shared bound store, so `stores_all_six_classes`' `style/tone`
    // would satisfy it for the wrong reason if the two collided.
    let result = tool
        .execute(json!({"class": "style", "key": "readonly_tone", "value": "formal"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(stored("pinned/style/readonly_tone").await.is_none());
}
