use super::*;
use crate::openhuman::agent::hooks::ToolCallRecord;
use crate::openhuman::memory::tool_memory::test_helpers::MockMemory;
use crate::openhuman::memory::tool_memory::tool_memory_store;

fn ctx_with(message: &str, tool_calls: Vec<ToolCallRecord>) -> TurnContext {
    TurnContext {
        user_message: message.into(),
        assistant_response: "ok".into(),
        tool_calls,
        turn_duration_ms: 1,
        session_id: None,
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    }
}

fn call(name: &str, success: bool) -> ToolCallRecord {
    ToolCallRecord {
        name: name.into(),
        arguments: serde_json::json!({}),
        success,
        output_summary: if success {
            "ok".into()
        } else {
            "permission denied".into()
        },
        duration_ms: 10,
    }
}

#[test]
fn extract_user_edicts_picks_up_never_phrase() {
    let edicts = ToolMemoryCaptureHook::extract_user_edicts(
        "Never email Sarah at sarah@example.com — she does not want updates.",
        &[call("send_email", true)],
    );
    assert!(!edicts.is_empty(), "expected at least one captured edict");
    let (tool, body) = &edicts[0];
    assert_eq!(
        tool, "send_email",
        "should map 'email' alias to send_email tool"
    );
    assert!(body.to_lowercase().contains("never email"));
}

#[test]
fn extract_user_edicts_handles_dont_and_stop_phrases() {
    let edicts = ToolMemoryCaptureHook::extract_user_edicts(
        "Don't run shell commands with sudo. Stop using browser for that.",
        &[call("shell", true), call("browser", true)],
    );
    assert_eq!(edicts.len(), 2, "should capture each imperative separately");
}

#[test]
fn extract_user_edicts_returns_empty_when_no_edict_present() {
    let edicts = ToolMemoryCaptureHook::extract_user_edicts(
        "Send Sarah an update when you can.",
        &[call("send_email", true)],
    );
    assert!(edicts.is_empty());
}

#[test]
fn extract_user_edicts_falls_back_to_first_tool_when_no_alias_match() {
    let edicts = ToolMemoryCaptureHook::extract_user_edicts(
        "Never do that automatically.",
        &[call("calendar", true)],
    );
    assert_eq!(edicts.len(), 1);
    assert_eq!(edicts[0].0, "calendar");
}

#[test]
fn extract_user_edicts_uses_sentinel_when_no_tools_ran() {
    let edicts = ToolMemoryCaptureHook::extract_user_edicts("Never do that.", &[]);
    assert_eq!(edicts.len(), 1);
    assert_eq!(edicts[0].0, "__unscoped__");
}

#[test]
fn extract_repeated_failures_needs_two_or_more_failures() {
    let observations = ToolMemoryCaptureHook::extract_repeated_failures(&[
        call("shell", false),
        call("shell", false),
        call("shell", true),
    ]);
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].0, "shell");
    assert!(observations[0].1.contains("failed 2 times"));
}

#[test]
fn extract_repeated_failures_ignores_single_failures() {
    let observations = ToolMemoryCaptureHook::extract_repeated_failures(&[call("shell", false)]);
    assert!(observations.is_empty());
}

#[tokio::test]
async fn on_turn_complete_persists_critical_rule_for_user_edict() {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    let store = tool_memory_store(memory.clone());
    let hook = ToolMemoryCaptureHook::from_store(store.clone(), true);

    hook.on_turn_complete(&ctx_with(
        "Never email Sarah — she opted out.",
        vec![call("send_email", true)],
    ))
    .await
    .unwrap();

    let rules = store.list_rules("send_email").await.unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].priority, ToolMemoryPriority::Critical);
    assert_eq!(rules[0].source, ToolMemorySource::UserExplicit);
    assert!(rules[0].tags.contains(&"user-edict".to_string()));
}

#[tokio::test]
async fn on_turn_complete_no_op_when_disabled() {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    let store = tool_memory_store(memory.clone());
    let hook = ToolMemoryCaptureHook::from_store(store.clone(), false);
    hook.on_turn_complete(&ctx_with(
        "Never email Sarah.",
        vec![call("send_email", true)],
    ))
    .await
    .unwrap();
    assert!(store.list_rules("send_email").await.unwrap().is_empty());
}

/// Safety case (AC #5): "never email Sarah" flows end-to-end from
/// a user utterance → captured as a Critical rule → surfaces in
/// the prompt-injection block.
#[tokio::test]
async fn safety_case_never_email_sarah_pins_into_prompt_block() {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    let store = tool_memory_store(memory.clone());
    let hook = ToolMemoryCaptureHook::from_store(store.clone(), true);

    // 1. Capture the edict from a normal user turn.
    hook.on_turn_complete(&ctx_with(
        "Never email Sarah at sarah@example.com.",
        vec![call("send_email", true)],
    ))
    .await
    .unwrap();

    // 2. The rule lands in the tool-scoped namespace with Critical
    //    priority — distinct from `tool_effectiveness` / global.
    let stored = store.list_rules("send_email").await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].priority, ToolMemoryPriority::Critical);

    // 3. `rules_for_prompt` pulls it eagerly so the session builder
    //    can pin it into the (compression-resistant) system prompt.
    let prompt = store
        .rules_for_prompt(&["send_email".to_string()])
        .await
        .unwrap();
    assert!(prompt.contains_key("send_email"));

    // 4. The rendered block is non-empty and mentions the edict
    //    verbatim — the exact bytes the safety pipeline puts in
    //    front of the agent on every subsequent turn.
    let mut flat: Vec<_> = prompt.into_values().flatten().collect();
    flat.sort_by(|a, b| b.priority.cmp(&a.priority));
    let rendered = crate::openhuman::memory::tool_memory::prompt::ToolMemoryRulesSection::new(flat)
        .rendered()
        .to_string();
    assert!(rendered.contains("Never email Sarah"));
    assert!(rendered.contains("**[critical]**"));
}

#[tokio::test]
async fn on_turn_complete_records_repeated_failure_observation() {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    let store = tool_memory_store(memory.clone());
    let hook = ToolMemoryCaptureHook::from_store(store.clone(), true);
    hook.on_turn_complete(&ctx_with(
        "Try again",
        vec![call("shell", false), call("shell", false)],
    ))
    .await
    .unwrap();
    let rules = store.list_rules("shell").await.unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].priority, ToolMemoryPriority::Normal);
    assert_eq!(rules[0].source, ToolMemorySource::PostTurn);
    assert!(rules[0].tags.contains(&"repeated-failure".to_string()));
}
