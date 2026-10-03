use super::*;

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
