use super::*;
use crate::openhuman::agent::experience::store::AgentExperienceStore;
use crate::openhuman::agent::experience::types::ExperienceOutcome;
use crate::openhuman::agent::hooks::{PostTurnHook, ToolCallRecord, TurnContext};
use crate::openhuman::memory::tool_memory::test_helpers::MockMemory;
use crate::openhuman::memory::Memory;
use std::sync::Arc;

fn ctx_with(tool_calls: Vec<ToolCallRecord>) -> TurnContext {
    TurnContext {
        user_message: "Search the repository docs before opening the target file.".into(),
        assistant_response: "I found the docs and used the target file.".into(),
        tool_calls,
        turn_duration_ms: 1200,
        session_id: Some("session-1".into()),
        agent_id: Some("orchestrator".into()),
        entrypoint: Some("web_channel".into()),
        iteration_count: 2,
    }
}

fn call(name: &str, success: bool, output_summary: &str) -> ToolCallRecord {
    ToolCallRecord {
        name: name.into(),
        arguments: serde_json::json!({}),
        success,
        output_summary: output_summary.into(),
        duration_ms: 10,
    }
}

#[test]
fn extract_candidates_records_successful_multi_tool_sequence() {
    let ctx = ctx_with(vec![
        call("grep", true, "grep: ok (20 chars)"),
        call("file_read", true, "file_read: ok (100 chars)"),
    ]);

    let candidates = AgentExperienceCaptureHook::extract_candidates(&ctx);

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].outcome, ExperienceOutcome::Success);
    assert_eq!(candidates[0].tool_sequence, vec!["grep", "file_read"]);
    assert_eq!(candidates[0].agent_id.as_deref(), Some("orchestrator"));
    assert_eq!(candidates[0].entrypoint.as_deref(), Some("web_channel"));
    assert!(candidates[0].lesson.contains("grep -> file_read"));
    assert!(candidates[0].tags.contains(&"multi-tool-success".into()));
}

#[test]
fn extract_candidates_records_repeated_failures() {
    let ctx = ctx_with(vec![
        call("shell", false, "shell: failed (permission_denied)"),
        call("shell", false, "shell: failed (permission_denied)"),
        call("grep", true, "grep: ok (10 chars)"),
    ]);

    let candidates = AgentExperienceCaptureHook::extract_candidates(&ctx);
    let repeated_failure = candidates
        .iter()
        .find(|candidate| candidate.tags.contains(&"repeated-failure".into()))
        .expect("repeated failure candidate");

    assert_eq!(repeated_failure.outcome, ExperienceOutcome::Failure);
    assert_eq!(
        repeated_failure.error_class.as_deref(),
        Some("permission_denied")
    );
    assert!(repeated_failure.lesson.contains("shell failed 2 times"));
    assert!(repeated_failure.avoid_hint.is_some());
}

#[tokio::test]
async fn on_turn_complete_persists_candidates() {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    let store = AgentExperienceStore::new(memory.clone());
    let hook = AgentExperienceCaptureHook::from_store(store.clone(), true);

    hook.on_turn_complete(&ctx_with(vec![
        call("grep", true, "grep: ok (20 chars)"),
        call("file_read", true, "file_read: ok (100 chars)"),
    ]))
    .await
    .unwrap();

    let stored = store.list().await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].outcome, ExperienceOutcome::Success);
    assert_eq!(stored[0].agent_id.as_deref(), Some("orchestrator"));
    assert_eq!(stored[0].entrypoint.as_deref(), Some("web_channel"));
    // Profile-less hook leaves records unstamped (shared/legacy).
    assert_eq!(stored[0].profile_id, None);
}

#[tokio::test]
async fn on_turn_complete_stamps_active_profile() {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    let hook =
        AgentExperienceCaptureHook::with_profile(memory.clone(), true, Some("alice".to_string()));

    hook.on_turn_complete(&ctx_with(vec![
        call("grep", true, "grep: ok (20 chars)"),
        call("file_read", true, "file_read: ok (100 chars)"),
    ]))
    .await
    .unwrap();

    let stored = AgentExperienceStore::new(memory).list().await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored[0].profile_id.as_deref(),
        Some("alice"),
        "captured record must be stamped with the active profile id"
    );
}

#[tokio::test]
async fn identical_candidates_under_different_profiles_do_not_collide() {
    // Alice and Bob learn the same task/tool/outcome triple. Their records
    // must land under distinct storage keys so neither overwrites the other,
    // and the profile-less (None) key must match the legacy derivation.
    let calls = || {
        vec![
            call("grep", true, "grep: ok (20 chars)"),
            call("file_read", true, "file_read: ok (100 chars)"),
        ]
    };

    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    AgentExperienceCaptureHook::with_profile(memory.clone(), true, Some("alice".to_string()))
        .on_turn_complete(&ctx_with(calls()))
        .await
        .unwrap();
    AgentExperienceCaptureHook::with_profile(memory.clone(), true, Some("bob".to_string()))
        .on_turn_complete(&ctx_with(calls()))
        .await
        .unwrap();
    AgentExperienceCaptureHook::new(memory.clone(), true)
        .on_turn_complete(&ctx_with(calls()))
        .await
        .unwrap();

    let stored = AgentExperienceStore::new(memory).list().await.unwrap();
    // Three distinct records rather than one repeatedly-overwritten key.
    assert_eq!(stored.len(), 3, "each profile keeps its own record");
    let ids: std::collections::HashSet<&str> = stored.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids.len(), 3, "the three storage keys must be distinct");

    // The profile-less record's key matches the legacy (profile-agnostic)
    // derivation for the same triple.
    let none_record = stored
        .iter()
        .find(|e| e.profile_id.is_none())
        .expect("a profile-less record");
    let legacy_id = stable_experience_id(
        &none_record.task_summary,
        &none_record.tool_sequence,
        none_record.outcome,
    );
    assert_eq!(none_record.id, legacy_id);
}
