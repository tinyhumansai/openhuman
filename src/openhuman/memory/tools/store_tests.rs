use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

use crate::openhuman::memory::api::types::MemoryEntry;

fn test_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy::default())
}

/// Read a memory back through the seam the tool wrote it through.
///
/// The tool holds no handle at all — it resolves the *bound* driver per
/// call — so a fixture store built here could never see the write: under a
/// real module the bound driver is the process-global test workspace, a
/// different store entirely. That is why no fixture handle exists in this
/// module, and why an absence assertion must not be made against one; it
/// would hold whether or not the refusal under test worked, which is worse
/// than no assertion at all.
///
/// The guard is the tool's own door, so a read through it proves the write
/// landed where a caller would look for it.
async fn stored(namespace: &str, key: &str) -> Option<MemoryEntry> {
    active_memory_guard()
        .await
        .expect("a bound memory guard")
        .get(namespace, key)
        .await
        .expect("read back through the guard")
}

#[test]
fn name_and_schema() {
    let tool = MemoryStoreTool::new(test_security());
    assert_eq!(tool.name(), "memory_store");
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["key"].is_object());
    assert!(schema["properties"]["content"].is_object());
    // The memory protocol (#4116) must be stated up front so the model recalls
    // for dedupe before writing and reconciles the index after.
    let desc = tool.description();
    assert!(
        desc.contains("memory_recall") && desc.contains("update_memory_md"),
        "memory_store description must state the read→dedupe→write→update contract: {desc}"
    );
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn store_core() {
    let tool = MemoryStoreTool::new(test_security());
    let result = tool
        .execute(json!({"namespace": "global", "key": "lang", "content": "Prefers Rust"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("lang"));

    let entry = stored("global", "lang").await;
    assert!(
        entry.is_some(),
        "the write is visible through the tool's own seam"
    );
    assert_eq!(entry.unwrap().content, "Prefers Rust");
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn store_with_category() {
    let tool = MemoryStoreTool::new(test_security());
    let result = tool
            .execute(
                json!({"namespace": "global", "key": "note", "content": "Fixed bug", "category": "daily"}),
            )
            .await
            .unwrap();
    assert!(!result.is_error);
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn store_with_custom_category() {
    let tool = MemoryStoreTool::new(test_security());
    let result = tool
            .execute(
                json!({"namespace": "global", "key": "proj_note", "content": "Uses async runtime", "category": "project"}),
            )
            .await
            .unwrap();
    assert!(!result.is_error);

    let entry = stored("global", "proj_note")
        .await
        .expect("the stored memory is readable through the guard");
    assert_eq!(entry.content, "Uses async runtime");
    assert_eq!(entry.category, MemoryCategory::Custom("project".into()));
}

/// Regression: a `custom:<name>` wire value (the form `memory_recall` and
/// `Display` now emit) must store as `Custom("<name>")`, not the
/// double-prefixed `Custom("custom:<name>")` — otherwise it would `Display`
/// as `custom:custom:<name>` and stop matching the original category.
#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn store_strips_custom_prefix_from_wire_category() {
    let tool = MemoryStoreTool::new(test_security());
    let result = tool
        .execute(json!({
            "namespace": "global",
            "key": "wire_prefixed_note",
            "content": "Uses async runtime",
            "category": "custom:project"
        }))
        .await
        .unwrap();
    assert!(!result.is_error);

    let entry = stored("global", "wire_prefixed_note")
        .await
        .expect("the stored memory is readable through the guard");
    assert_eq!(
        entry.category,
        MemoryCategory::Custom("project".into()),
        "the `custom:` wire prefix must be stripped, not double-stored"
    );
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn store_rejects_secret_like_content() {
    let tool = MemoryStoreTool::new(test_security());
    let result = tool
        .execute(json!({
            "namespace": "global",
            "key": "api",
            "content": "api_key=sk-123456789012345678901234567890"
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("looks like a secret"));
    // Through the guard — see `stored`: an absence assertion against a
    // store the tool never writes to holds whether or not the refusal
    // worked, which makes it worse than no assertion at all.
    assert!(stored("global", "api").await.is_none());
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn store_without_key_files_under_a_derived_key() {
    // "remember X" carries no key; the tool derives one instead of refusing
    // (#6048), and files under the default scope `memory_recall` reads back.
    let tool = MemoryStoreTool::new(test_security());
    let result = tool.execute(json!({"content": "no key"})).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    assert!(
        stored(DEFAULT_AGENT_MEMORY_NAMESPACE, &derive_key("no key"))
            .await
            .is_some()
    );
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn store_missing_content() {
    let tool = MemoryStoreTool::new(test_security());
    let result = tool.execute(json!({"key": "no_content"})).await;
    assert!(result.is_err());
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn store_blocked_in_readonly_mode() {
    let readonly = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let tool = MemoryStoreTool::new(readonly);
    let result = tool
        .execute(json!({"namespace": "global", "key": "readonly_lang", "content": "Prefers Rust"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only mode"));
    assert!(stored("global", "readonly_lang").await.is_none());
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn store_blocked_when_rate_limited() {
    let limited = Arc::new(SecurityPolicy {
        max_actions_per_hour: 0,
        ..SecurityPolicy::default()
    });
    let tool = MemoryStoreTool::new(limited);
    let result = tool
        .execute(
            json!({"namespace": "global", "key": "ratelimited_lang", "content": "Prefers Rust"}),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Rate limit exceeded"));
    assert!(stored("global", "ratelimited_lang").await.is_none());
}

// ── Argument resolution (#6048) — pure, no store needed ──────────────────────

/// Only the content is required: a one-line "remember X" must not send the
/// model off to invent a namespace and a key first.
#[test]
fn schema_requires_only_the_content() {
    let schema = MemoryStoreTool::new(test_security()).parameters_schema();
    assert_eq!(schema["required"], json!(["content"]));
    assert!(schema["properties"]["namespace"].is_object());
    assert!(schema["properties"]["key"].is_object());
}

#[test]
fn resolve_namespace_defaults_when_absent() {
    let namespace = resolve_namespace(&json!({"content": "x"})).unwrap();
    assert_eq!(namespace, DEFAULT_AGENT_MEMORY_NAMESPACE);
}

#[test]
fn resolve_namespace_keeps_an_explicit_one_trimmed() {
    let namespace =
        resolve_namespace(&json!({"namespace": " skill-gmail ", "content": "x"})).unwrap();
    assert_eq!(namespace, "skill-gmail");
}

#[test]
fn resolve_namespace_leaves_an_explicit_empty_one_for_the_caller_to_refuse() {
    // Empty is not "absent": the caller reports it rather than widening the
    // write to the default scope behind the model's back.
    let namespace = resolve_namespace(&json!({"namespace": "   ", "content": "x"})).unwrap();
    assert!(namespace.is_empty());
}

#[test]
fn resolve_namespace_rejects_a_non_string_namespace() {
    assert!(resolve_namespace(&json!({"namespace": null, "content": "x"})).is_err());
    assert!(resolve_namespace(&json!({"namespace": 7, "content": "x"})).is_err());
    assert!(resolve_namespace(&json!({"namespace": ["a"], "content": "x"})).is_err());
}

#[test]
fn resolve_key_keeps_an_explicit_one_trimmed() {
    let key = resolve_key(&json!({"key": " next_scrum ", "content": "x"}), "x").unwrap();
    assert_eq!(key, "next_scrum");
}

#[test]
fn resolve_key_rejects_a_non_string_key() {
    assert!(resolve_key(&json!({"key": null, "content": "x"}), "x").is_err());
    assert!(resolve_key(&json!({"key": 3, "content": "x"}), "x").is_err());
}

#[test]
fn resolve_key_derives_one_from_the_content_when_absent() {
    let content = "Next scrum meeting is on 10 September, 10am.";
    let key = resolve_key(&json!({"content": content}), content).unwrap();
    assert_eq!(key, derive_key(content));
    // Readable stem from the first words, then a short hash of the whole text.
    assert!(
        key.starts_with("next_scrum_meeting_is_on_10_"),
        "stem must come from the content's first words: {key}"
    );
    let suffix = key.rsplit('_').next().unwrap();
    assert_eq!(
        suffix.len(),
        DERIVED_KEY_HASH_CHARS,
        "the digest suffix is what keeps two notes with the same opening words apart: {key}"
    );
    assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()), "{key}");
}

#[test]
fn derive_key_is_deterministic_and_content_sensitive() {
    let a = "next scrum meeting on 10 september";
    assert_eq!(
        derive_key(a),
        derive_key(a),
        "re-saving must overwrite, not duplicate"
    );
    assert_ne!(
        derive_key(a),
        derive_key("next scrum meeting on 17 september"),
        "a different fact must not overwrite the first"
    );
    // Same opening words, different tail: the hash keeps them apart.
    assert_ne!(
        derive_key("the meeting with sam is about the budget"),
        derive_key("the meeting with sam is about the roadmap")
    );
    // Whitespace around the text is not part of the identity.
    assert_eq!(derive_key(a), derive_key(&format!("  {a}\n")));
}

#[test]
fn derive_key_falls_back_when_the_content_has_no_words() {
    let key = derive_key("!!! ???");
    assert!(key.starts_with("note_"), "{key}");
}

#[test]
fn derive_key_caps_the_stem() {
    let content = "a".repeat(200);
    let key = derive_key(&content);
    let stem = key.rsplit_once('_').map(|(stem, _)| stem).unwrap();
    assert_eq!(stem.chars().count(), DERIVED_KEY_STEM_CHARS);
}

/// A shared stem is the normal case, so the suffix carries the whole burden of
/// telling two notes apart — at 24 bits that failed within a few thousand
/// writes, and the loser was silently overwritten (review finding).
#[test]
fn derive_key_suffix_is_wide_enough_to_separate_a_shared_stem() {
    assert!(
        DERIVED_KEY_HASH_CHARS >= 16,
        "a derived key needs at least 64 bits of digest"
    );
    let a = derive_key("the meeting with sam is about the budget for next quarter");
    let b = derive_key("the meeting with sam is about the roadmap for next quarter");
    let stem = |key: &str| key.rsplit_once('_').map(|(s, _)| s.to_string()).unwrap();
    assert_eq!(
        stem(&a),
        stem(&b),
        "the fixture must share a stem to be meaningful"
    );
    assert_ne!(a, b, "a shared stem must still yield distinct keys");
}
