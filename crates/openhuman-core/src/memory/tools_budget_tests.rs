use super::*;
use crate::memory::test_fixtures::{bind_reference, config_in, stored};
use std::collections::HashSet;
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_harness::ids::CallId;
use tinyagents_harness::tool::ToolExecutionContext;
use tinymemory_api::MetaFilter;

#[tokio::test]
async fn learn_is_not_capped_but_reads_are_and_recover_on_the_next_chat_turn() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let tool = MemoryTool::new(Arc::new(config.clone()));
    let first: RunContext = RunContext::new(RunConfig::new("first"), ());
    let next: RunContext = RunContext::new(RunConfig::new("next"), ());
    // Eleven learns in one run, as one parallel batch sends them (#7717):
    // writes are local queue entries and never spend the read budget.
    for i in 0..11 {
        let context =
            ToolExecutionContext::from_run_context(&first, CallId::new(format!("learn-{i}")));
        let result = tool
            .execute_with_context(
                json!({"action":"learn","text":format!("fact {i}")}),
                ToolCallOptions::default(),
                Some(&context),
            )
            .await
            .unwrap();
        assert!(!result.is_error, "learn {i}: {}", result.text());
    }
    super::super::tool_writes::drain(&config).await;
    assert_eq!(stored(&engine, MetaFilter::default()).await.len(), 11);
    // Reads still stop at the cap, refused as the budget's own verdict.
    for i in 0..9 {
        let context =
            ToolExecutionContext::from_run_context(&first, CallId::new(format!("fetch-{i}")));
        let result = tool
            .execute_with_context(
                json!({"action":"fetch","query":"fact"}),
                ToolCallOptions::default(),
                Some(&context),
            )
            .await
            .unwrap();
        assert_eq!(result.is_error, i == 8, "fetch {i}: {}", result.text());
        assert_eq!(is_budget_refusal(&result), i == 8);
        if i == 8 {
            assert_eq!(
                result.metadata,
                Some(json!({"openhuman.memory_budget": "refused"}))
            );
        }
    }
    let context = ToolExecutionContext::from_run_context(&next, CallId::new("next-call"));
    let result = tool
        .execute_with_context(
            json!({"action":"fetch","query":"fact"}),
            ToolCallOptions::default(),
            Some(&context),
        )
        .await
        .unwrap();
    assert!(!result.is_error, "a new chat turn gets a fresh read budget");
}

#[tokio::test]
async fn identical_learnings_in_one_run_deduplicate() {
    use tinyagents_harness::context::{RunConfig, RunContext};
    use tinyagents_harness::ids::CallId;
    use tinyagents_harness::tool::ToolExecutionContext;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let tool = MemoryTool::new(Arc::new(config.clone()));
    let first: RunContext = RunContext::new(RunConfig::new("first-harness-run"), ());
    let next: RunContext = RunContext::new(RunConfig::new("next-harness-run"), ());
    let args = json!({"action":"learn","text":"Prefers tea"});
    let mut ids = HashSet::new();
    for i in 0..9 {
        let context =
            ToolExecutionContext::from_run_context(&first, CallId::new(format!("call-{i}")));
        let result = tool
            .execute_with_context(args.clone(), ToolCallOptions::default(), Some(&context))
            .await
            .unwrap();
        assert!(!result.is_error, "learn {i}: {}", result.text());
        let value: Value = serde_json::from_str(&result.text()).unwrap();
        ids.insert(value["id"].as_str().unwrap().to_string());
    }
    assert_eq!(
        ids.len(),
        1,
        "invocation IDs must not create duplicate learnings"
    );
    let context = ToolExecutionContext::from_run_context(&next, CallId::new("next-call"));
    let result = tool
        .execute_with_context(args, ToolCallOptions::default(), Some(&context))
        .await
        .unwrap();
    assert!(!result.is_error, "a learn in a new harness run is accepted");
    super::super::tool_writes::drain(&config).await;
    assert_eq!(stored(&engine, MetaFilter::default()).await.len(), 1);
}
