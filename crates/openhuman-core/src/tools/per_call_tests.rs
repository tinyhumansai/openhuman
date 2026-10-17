use super::*;
use crate::agent::tinyagents::host::run_context::tool_context_with_approval;
use crate::security::approval::ApprovalTurnContext;
use serde_json::json;

/// Reports the chat it was built for, so a test can see which call built it.
struct Probe {
    built_for: Option<ApprovalChatContext>,
}

#[async_trait]
impl Tool for Probe {
    fn name(&self) -> &str {
        "probe"
    }

    fn description(&self) -> &str {
        "reports the chat it was built for"
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn external_effect(&self) -> bool {
        true
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        let thread = self
            .built_for
            .as_ref()
            .map_or_else(|| "none".to_string(), |chat| chat.thread_id.clone());
        Ok(ToolResult::success(thread))
    }
}

fn chat(thread: &str) -> ApprovalChatContext {
    ApprovalChatContext {
        thread_id: thread.to_string(),
        client_id: format!("client-{thread}"),
        request_id: None,
    }
}

fn tool() -> PerCallTool<Probe> {
    PerCallTool::new(|chat, _ctx| Probe {
        built_for: chat.cloned(),
    })
}

#[tokio::test]
async fn each_call_is_built_around_its_own_turns_chat() {
    let tool = tool();
    let a = tool_context_with_approval(ApprovalTurnContext::chat(chat("thread-a")));
    let b = tool_context_with_approval(ApprovalTurnContext::chat(chat("thread-b")));

    // Two turns running at once must not see each other's chat: the chat is
    // read from each call's own context, not from anything the tool remembers.
    let (first, second) = tokio::join!(
        tool.execute_with_context(json!({}), ToolCallOptions::default(), Some(&a)),
        tool.execute_with_context(json!({}), ToolCallOptions::default(), Some(&b)),
    );
    assert_eq!(first.unwrap().text(), "thread-a");
    assert_eq!(second.unwrap().text(), "thread-b");
}

#[tokio::test]
async fn a_call_with_no_run_context_is_built_without_a_chat() {
    let tool = tool();
    assert_eq!(tool.execute(json!({})).await.unwrap().text(), "none");
    assert_eq!(
        tool.execute_with_options(json!({}), ToolCallOptions::default())
            .await
            .unwrap()
            .text(),
        "none"
    );
    let non_chat = tool_context_with_approval(ApprovalTurnContext::default());
    assert_eq!(
        tool.execute_with_context(json!({}), ToolCallOptions::default(), Some(&non_chat))
            .await
            .unwrap()
            .text(),
        "none"
    );
}

#[tokio::test]
async fn a_chat_survives_await_points_inside_the_call() {
    let tool = tool();
    let context = tool_context_with_approval(ApprovalTurnContext::chat(chat("thread-c")));
    let result = tokio::spawn(async move {
        tokio::task::yield_now().await;
        tool.execute_with_context(json!({}), ToolCallOptions::default(), Some(&context))
            .await
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.text(), "thread-c");
}

#[test]
fn static_metadata_comes_from_the_prototype() {
    let tool = tool();
    assert_eq!(tool.name(), "probe");
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert!(tool.external_effect());
}

#[tokio::test]
async fn each_call_is_built_around_its_runs_dispatch_context() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    use std::sync::{Arc, Mutex};

    let seen: Arc<Mutex<Vec<Option<Arc<CoreContext>>>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let tool = PerCallTool::new(move |chat, ctx: &DispatchCtx| {
        sink.lock().unwrap().push(ctx.core().cloned());
        Probe {
            built_for: chat.cloned(),
        }
    });
    seen.lock().unwrap().clear();

    let core = CoreContext::for_test(DomainSet::full(), None);
    let ambient = CoreContext::for_test(DomainSet::full(), None);
    let run =
        crate::agent::tinyagents::host::OpenHumanRunContext::new().with_core(Some(core.clone()));
    let run = run.into_tinyagents(tinyagents_harness::context::RunConfig::new("per-call-ctx"));
    let context = tinyagents_harness::tool::ToolExecutionContext::from_run_context(
        &run,
        tinyagents_harness::ids::CallId::new("per-call-ctx-call"),
    );

    CoreContext::scope(ambient, async {
        tool.execute_with_context(
            serde_json::json!({}),
            tinytools::ToolCallOptions::default(),
            Some(&context),
        )
        .await
        .unwrap();
    })
    .await;

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(Arc::ptr_eq(seen[0].as_ref().expect("bound"), &core));
}
