use super::*;
use crate::agent::tinyagents::middleware::embedder_hooks::EmbedderToolHooksMiddleware;

#[tokio::test]
async fn embedder_tool_hooks_post_use_replays_the_normalized_pre_call_arguments() {
    let pre = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let post = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mw = embedder_hook_mw(pre.clone(), post.clone(), false);

    let mut call = TaToolCall {
        id: "call-1".into(),
        name: "lookup".into(),
        arguments: json!({"id": 42}),
        invalid: None,
    };
    mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();

    let mut result = TaToolResult::success("found");
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("call-1", "lookup"),
        &mut result,
    )
    .await
    .unwrap();

    assert_eq!(pre.lock().unwrap().len(), 1, "one pre-use notification");
    let post = post.lock().unwrap();
    assert_eq!(post.len(), 1, "one post-use notification");
    let (tool, arguments, success, duration) = &post[0];
    assert_eq!(tool, "lookup");
    assert_eq!(
        *arguments,
        json!({"id": 42}),
        "post-use context must preserve the normalized pre-call arguments, not Null"
    );
    assert_eq!(*success, Some(true));
    assert_eq!(
        *duration, None,
        "canonical ToolResult carries no elapsed field"
    );
}

#[tokio::test]
async fn embedder_tool_hooks_veto_denies_the_call_and_skips_post_use() {
    let pre = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let post = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mw = embedder_hook_mw(pre.clone(), post.clone(), true);

    let mut call = TaToolCall {
        id: "call-2".into(),
        name: "rm".into(),
        arguments: json!({"path": "/"}),
        invalid: None,
    };
    let error = mw
        .before_tool(&mut ctx(), &(), &mut call)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("vetoed"),
        "veto must surface as a tool error: {error}"
    );
    // The call was vetoed — no post-use event, and no cache entry leaks.
    assert_eq!(pre.lock().unwrap().len(), 1, "pre-use hook still observed");
    assert!(
        post.lock().unwrap().is_empty(),
        "no post-use for a vetoed call"
    );
    assert!(
        mw.arguments_by_call_id.lock().unwrap().is_empty(),
        "a vetoed call must not leave a cached argument entry"
    );
}

#[tokio::test]
async fn embedder_tool_hooks_post_use_without_pre_call_falls_back_to_null() {
    let pre = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let post = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mw = embedder_hook_mw(pre.clone(), post.clone(), false);

    // A result with no matching `before_tool` (defensive path) must not panic
    // and falls back to `Null`, the pre-fix behaviour.
    let mut result = TaToolResult::success("found");
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("orphan", "lookup"),
        &mut result,
    )
    .await
    .unwrap();
    let post = post.lock().unwrap();
    assert_eq!(post.len(), 1);
    assert_eq!(post[0].1, serde_json::Value::Null);
    assert_eq!(post[0].2, Some(true));
}

fn nested_call(name: &str) -> TaToolCall {
    TaToolCall {
        id: "parent/1".into(),
        name: name.into(),
        arguments: json!({"path": "/"}),
        invalid: None,
    }
}

/// A hook whose pre-tool decision is fixed, to exercise every arm.
struct FixedDecisionHook(crate::agent::hooks::ToolHookDecision);

#[async_trait]
impl crate::agent::hooks::ToolHook for FixedDecisionHook {
    fn name(&self) -> &str {
        "fixed"
    }
    async fn before_tool(
        &self,
        _context: &crate::agent::hooks::ToolHookContext,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn after_tool(
        &self,
        _context: &crate::agent::hooks::ToolHookContext,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn before_tool_decision(
        &self,
        _context: &crate::agent::hooks::ToolHookContext,
    ) -> crate::agent::hooks::ToolHookDecision {
        self.0.clone()
    }
}

#[tokio::test]
async fn embedder_tool_hooks_nested_call_is_admitted_when_hooks_proceed() {
    let pre = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let post = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mw = embedder_hook_mw(pre.clone(), post, false);
    mw.check_nested_tool(&ctx(), &(), &nested_call("lookup"))
        .await
        .unwrap();
    assert_eq!(
        pre.lock().unwrap().len(),
        1,
        "hook consulted for nested call"
    );
    assert!(
        mw.arguments_by_call_id.lock().unwrap().is_empty(),
        "a nested check caches nothing"
    );
}

#[tokio::test]
async fn embedder_tool_hooks_nested_call_honours_a_veto() {
    let pre = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let post = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mw = embedder_hook_mw(pre, post, true);
    let error = mw
        .check_nested_tool(&ctx(), &(), &nested_call("rm"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("vetoed"), "{error}");
}

#[tokio::test]
async fn embedder_tool_hooks_nested_call_refuses_ask_and_rewrite() {
    use crate::agent::hooks::ToolHookDecision;
    for decision in [
        ToolHookDecision::Ask("confirm".into()),
        ToolHookDecision::ProceedWith(json!({"path": "."})),
    ] {
        let mw = EmbedderToolHooksMiddleware::new(vec![std::sync::Arc::new(FixedDecisionHook(
            decision,
        ))]);
        let error = mw
            .check_nested_tool(&ctx(), &(), &nested_call("write"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("refused nested call"), "{error}");
    }
}
