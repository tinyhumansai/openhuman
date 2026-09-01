//! Unit tests for the host-side rule store.
//!
//! These pin the parts that decide **where a rule lands** and **which rules
//! reach the prompt** — the namespace, the key, the priority ordering, and the
//! Critical-survives-the-cap rule. That is deliberately the set a divergence
//! from the module's own writer would show up in first: both sides build the
//! namespace and key from the contract, so a test that asserts the bytes here
//! is asserting the shared convention rather than a local restatement of it.

use super::*;
use crate::openhuman::memory::api::tool_memory::ToolMemorySource;
use crate::openhuman::memory::tool_memory::test_helpers::MockMemory;

fn store() -> ToolMemoryStore {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    ToolMemoryStore::new(memory)
}

#[tokio::test]
async fn record_lands_in_the_tool_scoped_namespace_under_a_rule_key() {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    let store = ToolMemoryStore::new(memory.clone());

    let stored = store
        .record(
            "  Send_Email  ",
            "never email Sarah",
            ToolMemoryPriority::Critical,
            ToolMemorySource::UserExplicit,
            vec!["user-edict".to_string()],
        )
        .await
        .unwrap();

    // The write normalises the tool name, so namespace and display identity
    // cannot skew.
    assert_eq!(stored.tool_name, "send_email");

    let entries = memory
        .list(Some("tool-send_email"), None, None)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1, "one rule, in the tool-scoped namespace");
    assert_eq!(entries[0].key, format!("rule/{}", stored.id));

    // And a read-back with the caller's *raw* name still resolves.
    let rules = store.list_rules("  Send_Email  ").await.unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].rule, "never email Sarah");
}

#[tokio::test]
async fn put_rule_preserves_created_at_on_upsert_and_refreshes_updated_at() {
    let store = store();
    let first = store
        .record(
            "shell",
            "prefer rg over grep",
            ToolMemoryPriority::Normal,
            ToolMemorySource::PostTurn,
            Vec::new(),
        )
        .await
        .unwrap();

    let mut amended = first.clone();
    amended.rule = "prefer rg over grep, always".to_string();
    amended.created_at = "1999-01-01T00:00:00+00:00".to_string();
    let second = store.put_rule(amended).await.unwrap();

    assert_eq!(
        second.created_at, first.created_at,
        "an upsert keeps the original creation time, not the caller's claim"
    );
    assert!(second.updated_at >= first.updated_at);
    assert_eq!(
        store.list_rules("shell").await.unwrap().len(),
        1,
        "same id upserts rather than appending"
    );
}

#[tokio::test]
async fn put_rule_rejects_a_blank_tool_name_or_body() {
    let store = store();
    let mut blank_tool = ToolMemoryRule::new(
        "   ",
        "something",
        ToolMemoryPriority::Normal,
        ToolMemorySource::Programmatic,
    );
    blank_tool.tool_name = "   ".to_string();
    assert!(store.put_rule(blank_tool).await.is_err());

    let blank_body = ToolMemoryRule::new(
        "shell",
        "   ",
        ToolMemoryPriority::Normal,
        ToolMemorySource::Programmatic,
    );
    assert!(store.put_rule(blank_body).await.is_err());
}

#[tokio::test]
async fn list_rules_orders_by_priority_then_freshness() {
    let store = store();
    for (body, priority) in [
        ("normal one", ToolMemoryPriority::Normal),
        ("critical one", ToolMemoryPriority::Critical),
        ("high one", ToolMemoryPriority::High),
    ] {
        store
            .record(
                "shell",
                body,
                priority,
                ToolMemorySource::Programmatic,
                Vec::new(),
            )
            .await
            .unwrap();
    }

    let rules = store.list_rules("shell").await.unwrap();
    let priorities: Vec<_> = rules.iter().map(|r| r.priority).collect();
    assert_eq!(
        priorities,
        vec![
            ToolMemoryPriority::Critical,
            ToolMemoryPriority::High,
            ToolMemoryPriority::Normal,
        ]
    );
}

#[tokio::test]
async fn rules_for_prompt_keeps_only_eager_rules() {
    let store = store();
    store
        .record(
            "shell",
            "normal guidance",
            ToolMemoryPriority::Normal,
            ToolMemorySource::PostTurn,
            Vec::new(),
        )
        .await
        .unwrap();
    store
        .record(
            "shell",
            "critical constraint",
            ToolMemoryPriority::Critical,
            ToolMemorySource::UserExplicit,
            Vec::new(),
        )
        .await
        .unwrap();

    let grouped = store
        .rules_for_prompt(&["shell".to_string()])
        .await
        .unwrap();
    let rules = grouped.get("shell").expect("shell has eager rules");
    assert_eq!(rules.len(), 1, "Normal rules are not eagerly surfaced");
    assert_eq!(rules[0].rule, "critical constraint");
}

/// The cap is a plain truncate at [`TOOL_MEMORY_PROMPT_CAP`], exactly as the
/// engine enforced it — Critical sorts first, so Criticals are the LAST to
/// fall, but past the cap they do fall. The port briefly kept every Critical
/// (`cap.max(critical_count)`), which was a silent behaviour change in a
/// behaviour-pinned move; this test now pins the engine's trade-off instead,
/// the same one the engine documents on the constant itself.
#[tokio::test]
async fn rules_for_prompt_truncates_at_the_cap_exactly_as_the_engine_did() {
    let store = store();
    let critical_count = TOOL_MEMORY_PROMPT_CAP + 5;
    for i in 0..critical_count {
        store
            .record(
                "shell",
                &format!("critical {i}"),
                ToolMemoryPriority::Critical,
                ToolMemorySource::UserExplicit,
                Vec::new(),
            )
            .await
            .unwrap();
    }
    store
        .record(
            "shell",
            "high one",
            ToolMemoryPriority::High,
            ToolMemorySource::PostTurn,
            Vec::new(),
        )
        .await
        .unwrap();

    let grouped = store
        .rules_for_prompt(&["shell".to_string()])
        .await
        .unwrap();
    let rules = grouped.get("shell").expect("shell has eager rules");
    assert_eq!(
        rules.len(),
        TOOL_MEMORY_PROMPT_CAP,
        "the cap is a hard truncate, engine semantics"
    );
    assert!(
        rules
            .iter()
            .all(|r| r.priority == ToolMemoryPriority::Critical),
        "Critical sorts first, so the surviving page is all-Critical; the \
         overflow Criticals and the High are what fell"
    );
}

#[tokio::test]
async fn list_tool_names_skips_the_unscoped_sentinel_and_non_tool_namespaces() {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    let store = ToolMemoryStore::new(memory.clone());

    store
        .record(
            "shell",
            "a",
            ToolMemoryPriority::Normal,
            ToolMemorySource::Programmatic,
            Vec::new(),
        )
        .await
        .unwrap();
    store
        .record(
            UNSCOPED_TOOL,
            "an edict captured with no tool call",
            ToolMemoryPriority::Critical,
            ToolMemorySource::UserExplicit,
            Vec::new(),
        )
        .await
        .unwrap();
    memory
        .store(
            "global",
            "unrelated",
            "not a rule",
            MemoryCategory::Custom("other".into()),
            None,
        )
        .await
        .unwrap();

    assert_eq!(store.list_tool_names().await.unwrap(), vec!["shell"]);
}

/// An unscoped edict is still *stored* — it is only withheld from the prompt
/// prefetch, so the agent can refile it later.
#[tokio::test]
async fn the_unscoped_sentinel_is_withheld_from_prefetch_but_still_readable() {
    let store = store();
    store
        .record(
            UNSCOPED_TOOL,
            "never do that",
            ToolMemoryPriority::Critical,
            ToolMemorySource::UserExplicit,
            Vec::new(),
        )
        .await
        .unwrap();

    assert_eq!(store.list_rules(UNSCOPED_TOOL).await.unwrap().len(), 1);
    assert!(
        store.rules_for_prompt(&[]).await.unwrap().is_empty(),
        "a whole-store prefetch must not pin an unscoped edict against every tool"
    );
}

#[tokio::test]
async fn list_rules_skips_a_corrupt_row_rather_than_failing() {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    let store = ToolMemoryStore::new(memory.clone());
    store
        .record(
            "shell",
            "good rule",
            ToolMemoryPriority::Normal,
            ToolMemorySource::Programmatic,
            Vec::new(),
        )
        .await
        .unwrap();
    memory
        .store(
            "tool-shell",
            "rule/corrupt",
            "{not json",
            MemoryCategory::Custom("tool_memory".into()),
            None,
        )
        .await
        .unwrap();

    let rules = store.list_rules("shell").await.unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].rule, "good rule");
}
