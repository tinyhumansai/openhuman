use super::*;
use crate::memory::test_fixtures::config_in;

fn text(result: &ToolResult) -> String {
    result.text()
}

#[tokio::test]
async fn queued_writes_work_without_an_available_engine_but_not_when_explicitly_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    assert!(!crate::memory::engine::switched_off(&config));
    assert!(
        !crate::memory::engine::is_on(&config).await,
        "the fixture has no credential or installed engine"
    );
    let facts = CallFacts::gather(&config, None);
    let tool = MemoryTool::new(Arc::new(config.clone()));
    let learned = tool
        .execute(json!({"action":"learn","text":"queued while offline"}))
        .await
        .unwrap();
    let forgotten = tool
        .execute(json!({"action":"forget","ids":["offline-item"]}))
        .await
        .unwrap();
    assert!(!learned.is_error, "learn: {}", text(&learned));
    assert!(!forgotten.is_error, "forget: {}", text(&forgotten));
    let pending = crate::memory::tool_writes::tests::queued_for(&config, &facts).unwrap();
    assert_eq!(
        pending.len(),
        2,
        "both operations must be durable in the outbox"
    );
    assert!(matches!(
        &pending[0].operation,
        tinymemory_tools::pending::WriteOperation::Learn { .. }
    ));
    assert!(matches!(
        &pending[1].operation,
        tinymemory_tools::pending::WriteOperation::Forget { .. }
    ));
    assert!(pending
        .iter()
        .all(|entry| entry.scope_root == facts.reach.at));

    let mut disabled = config.clone();
    disabled.components.memory = false;
    let disabled_tool = MemoryTool::new(Arc::new(disabled.clone()));
    for args in [
        json!({"action":"learn","text":"must not queue"}),
        json!({"action":"forget","ids":["offline-item"]}),
    ] {
        let result = disabled_tool.execute(args).await.unwrap();
        assert!(result.is_error, "explicit disable must reject the write");
    }
    let after_disabled = crate::memory::tool_writes::tests::queued_for(
        &disabled,
        &CallFacts::gather(&disabled, None),
    )
    .unwrap();
    assert_eq!(after_disabled, pending, "disabled memory must not append");
}
