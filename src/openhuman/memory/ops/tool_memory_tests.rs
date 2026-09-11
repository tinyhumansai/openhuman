use super::*;
use crate::openhuman::memory::api::tool_memory::ToolMemoryPriority;

fn ensure_memory_client() {
    crate::openhuman::memory::ops::shared_memory_test_workspace();
}

fn unique_tool_name() -> String {
    format!(
        "toolmem_test_{}",
        &uuid::Uuid::new_v4().as_simple().to_string()[..12]
    )
}

#[tokio::test]
async fn tool_rule_put_get_list_and_delete_roundtrip() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    ensure_memory_client();
    let tool_name = unique_tool_name();

    let stored = tool_rule_put(ToolRulePutParams {
        tool_name: tool_name.clone(),
        rule: "Always ask before sending emails".into(),
        priority: None,
        source: None,
        tags: vec!["safety".into()],
        id: Some("   ".into()),
    })
    .await
    .expect("tool rule put")
    .value;

    assert_eq!(stored.tool_name, tool_name);
    assert_eq!(stored.priority, ToolMemoryPriority::Normal);
    assert_eq!(
        stored.source,
        crate::openhuman::memory::api::tool_memory::ToolMemorySource::Programmatic
    );
    assert_eq!(stored.tags, vec!["safety".to_string()]);
    assert!(
        !stored.id.trim().is_empty(),
        "blank id should be regenerated"
    );

    let fetched = tool_rule_get(ToolRuleRefParams {
        tool_name: stored.tool_name.clone(),
        id: stored.id.clone(),
    })
    .await
    .expect("tool rule get")
    .value
    .expect("stored rule should exist");
    assert_eq!(fetched.rule, "Always ask before sending emails");

    let listed = tool_rule_list(ToolRuleListParams {
        tool_name: stored.tool_name.clone(),
    })
    .await
    .expect("tool rule list")
    .value;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, stored.id);

    let deleted = tool_rule_delete(ToolRuleRefParams {
        tool_name: stored.tool_name.clone(),
        id: stored.id.clone(),
    })
    .await
    .expect("tool rule delete")
    .value;
    assert!(deleted);

    let after = tool_rule_get(ToolRuleRefParams {
        tool_name: stored.tool_name,
        id: stored.id,
    })
    .await
    .expect("tool rule get after delete");
    assert!(after.value.is_none());
}

#[tokio::test]
async fn tool_rules_for_prompt_sorts_by_priority_and_tool_name() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    ensure_memory_client();
    let primary_tool = unique_tool_name();
    let secondary_tool = unique_tool_name();

    let high = tool_rule_put(ToolRulePutParams {
        tool_name: primary_tool.clone(),
        rule: "Use the dry-run mode first".into(),
        priority: Some(ToolMemoryPriority::High),
        source: None,
        tags: vec![],
        id: None,
    })
    .await
    .expect("put high")
    .value;
    let normal = tool_rule_put(ToolRulePutParams {
        tool_name: secondary_tool.clone(),
        rule: "Log the final command".into(),
        priority: Some(ToolMemoryPriority::Normal),
        source: None,
        tags: vec![],
        id: None,
    })
    .await
    .expect("put normal")
    .value;

    let prompt = tool_rules_for_prompt(ToolRulesForPromptParams {
        tools: vec![secondary_tool.clone(), primary_tool.clone()],
    })
    .await
    .expect("rules for prompt")
    .value;

    assert_eq!(prompt.rules.len(), 1, "only eager rules should be included");
    assert_eq!(prompt.rules[0].id, high.id);
    assert!(prompt.rendered.contains(&primary_tool));
    assert!(prompt.rendered.contains("Use the dry-run mode first"));

    let json_rules = tool_rules_json(ToolRuleListParams {
        tool_name: secondary_tool.clone(),
    })
    .await
    .expect("tool rules json")
    .value;
    assert!(json_rules.is_array(), "tool rules json should be an array");
    assert!(json_rules
        .as_array()
        .expect("array")
        .iter()
        .any(|row| row["rule"] == "Log the final command"));

    let _ = tool_rule_delete(ToolRuleRefParams {
        tool_name: primary_tool,
        id: high.id,
    })
    .await;
    let _ = tool_rule_delete(ToolRuleRefParams {
        tool_name: secondary_tool,
        id: normal.id,
    })
    .await;
}

/// Host-shaped put and guarded list/delete compose the same module-backed
/// tool-memory API.
#[tokio::test]
async fn guarded_list_and_delete_share_the_store_with_host_shaped_put() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    ensure_memory_client();
    let tool_name = unique_tool_name();

    let stored = tool_rule_put(ToolRulePutParams {
        tool_name: tool_name.clone(),
        rule: "Prefer the guarded path".into(),
        priority: None,
        source: None,
        tags: vec![],
        id: None,
    })
    .await
    .expect("host-shaped put")
    .value;

    let listed = tool_rule_list(ToolRuleListParams {
        tool_name: tool_name.clone(),
    })
    .await
    .expect("guarded list")
    .value;
    assert_eq!(listed.len(), 1, "the guard must see the API write");
    assert_eq!(listed[0].id, stored.id);

    let deleted = tool_rule_delete(ToolRuleRefParams {
        tool_name: tool_name.clone(),
        id: stored.id.clone(),
    })
    .await
    .expect("guarded delete")
    .value;
    assert!(deleted);

    let remaining = tool_rule_list(ToolRuleListParams { tool_name })
        .await
        .expect("module-backed list")
        .value;
    assert!(
        remaining.is_empty(),
        "the module-backed provider must observe the guarded delete"
    );
}
