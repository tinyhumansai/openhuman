use super::*;

#[tokio::test]
async fn failed_memory_write_does_not_advance_the_protocol() {
    let mw = MemoryProtocolMiddleware::new();
    let failed = run_cycle(
        &mw,
        "memory_store",
        json!({}),
        "disk full",
        Some("disk full"),
    )
    .await;
    // A failed write is not annotated and leaves nothing pending, so a later
    // run-end sweep must not warn about a stale index.
    assert!(!result_text(&failed).contains(MEMORY_PROTOCOL_MARKER));
    let mut run = AgentRun::new();
    // after_agent is a no-op warn path; it must not error.
    mw.after_agent(&mut ctx(), &(), &mut run).await.unwrap();
}

#[tokio::test]
async fn second_write_without_an_update_flags_index_drift() {
    let mw = MemoryProtocolMiddleware::new();
    run_cycle(&mw, "memory_recall", json!({}), "checked", None).await;
    let first = run_cycle(&mw, "memory_store", json!({}), "a", None).await;
    assert!(!result_text(&first).contains("drifting"));

    // No update_memory_md between the two writes → the index is drifting.
    let second = run_cycle(&mw, "memory_store", json!({}), "b", None).await;
    assert!(
        result_text(&second).contains("drifting"),
        "a second unsynced write should flag index drift: {}",
        result_text(&second)
    );
}

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

// ── FinalCallWrapUpMiddleware (issue #6014) ──────────────────────────────────

fn sink_with(entries: &[(&str, &str)]) -> crate::agent::tinyagents::ToolOutcomeSink {
    std::sync::Arc::new(std::sync::Mutex::new(
        entries
            .iter()
            .map(|(id, content)| crate::agent::tinyagents::ToolCallOutcome {
                call_id: (*id).to_string(),
                name: "fetch".to_string(),
                success: true,
                content: (*content).to_string(),
            })
            .collect(),
    ))
}

/// A run with calls left is untouched: no instruction, and the tool belt intact.
#[tokio::test]
async fn wrap_up_leaves_a_call_with_budget_remaining_alone() {
    let mw = FinalCallWrapUpMiddleware::new("CONCLUDE NOW", sink_with(&[]), 0);
    let mut ctx = RunContext::new(
        RunConfig::new("mw-test").with_max_model_calls(5),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    );
    ctx.limits.record_model_call().unwrap();
    let mut request = ModelRequest {
        messages: vec![TaMessage::user("hi")],
        tools: vec![ToolSchema::new("echo", "echo", serde_json::json!({}))],
        ..Default::default()
    };

    mw.before_model(&mut ctx, &(), &mut request).await.unwrap();

    assert_eq!(
        request.messages.len(),
        1,
        "no instruction should be appended"
    );
    assert_eq!(request.tools.len(), 1, "the belt must stay intact mid-turn");
}

/// On the last permitted call the tools are withdrawn — structurally, not by
/// asking — and the wrap-up instruction is appended as the final turn.
#[tokio::test]
async fn wrap_up_withdraws_tools_and_appends_the_instruction_on_the_last_call() {
    let mw = FinalCallWrapUpMiddleware::new("CONCLUDE NOW", sink_with(&[]), 0);
    let mut ctx = RunContext::new(
        RunConfig::new("mw-test").with_max_model_calls(2),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    );
    ctx.limits.record_model_call().unwrap();
    ctx.limits.record_model_call().unwrap(); // now the final call
    let mut request = ModelRequest {
        messages: vec![TaMessage::user("hi")],
        tools: vec![ToolSchema::new("echo", "echo", serde_json::json!({}))],
        tool_choice: tinyinference_llm::model::ToolChoice::Required,
        ..Default::default()
    };

    mw.before_model(&mut ctx, &(), &mut request).await.unwrap();

    assert!(
        request.tools.is_empty(),
        "tools must be withdrawn, not discouraged"
    );
    assert!(
        matches!(
            request.tool_choice,
            tinyinference_llm::model::ToolChoice::None
        ),
        "a Required choice with no tools is a provider 400"
    );
    assert_eq!(
        request.messages.last().map(|m| m.text()),
        Some("CONCLUDE NOW".to_string()),
        "the instruction must be the final turn of the request"
    );
    assert!(mw.fired().load(std::sync::atomic::Ordering::SeqCst));
}

/// The concluding call gets back the results microcompact blanked — otherwise
/// it is asked to report findings it cannot read, which is the same
/// empty-handed answer the whole mechanism exists to prevent.
#[tokio::test]
async fn wrap_up_restores_tool_results_microcompact_cleared() {
    let mw = FinalCallWrapUpMiddleware::new(
        "CONCLUDE NOW",
        sink_with(&[
            ("call-old", "issue #41: auth bypass"),
            ("call-new", "issue #42: leak"),
        ]),
        // Unbounded: this case is about restoring what was cleared, not about
        // the budget that stops it (covered by its own test below).
        0,
    );
    let mut ctx = RunContext::new(
        RunConfig::new("mw-test").with_max_model_calls(2),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    );
    ctx.limits.record_model_call().unwrap();
    ctx.limits.record_model_call().unwrap();
    let mut request = ModelRequest {
        messages: vec![
            // The shape microcompact leaves behind: an older result blanked to
            // the placeholder, a recent one kept verbatim.
            TaMessage::tool("call-old", CLEARED_PLACEHOLDER),
            TaMessage::tool("call-new", "issue #42: leak"),
        ],
        ..Default::default()
    };

    mw.before_model(&mut ctx, &(), &mut request).await.unwrap();

    let bodies: Vec<String> = request.messages.iter().map(|m| m.text()).collect();
    assert!(
        bodies.iter().any(|b| b.contains("issue #41: auth bypass")),
        "the cleared result should be restored from the capture sink: {bodies:?}"
    );
    assert!(
        !bodies.iter().any(|b| b.trim() == CLEARED_PLACEHOLDER),
        "no placeholder should survive into the concluding call: {bodies:?}"
    );
}

/// A result the model legitimately saw in full is never rewritten, even when
/// the sink holds a different (e.g. later-truncated) copy for that id.
#[tokio::test]
async fn wrap_up_does_not_rewrite_a_result_that_was_never_cleared() {
    let mw =
        FinalCallWrapUpMiddleware::new("CONCLUDE NOW", sink_with(&[("call-1", "FROM SINK")]), 0);
    let mut ctx = RunContext::new(
        RunConfig::new("mw-test").with_max_model_calls(2),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    );
    ctx.limits.record_model_call().unwrap();
    ctx.limits.record_model_call().unwrap();
    let mut request = ModelRequest {
        messages: vec![TaMessage::tool("call-1", "IN THE TRANSCRIPT")],
        ..Default::default()
    };

    mw.before_model(&mut ctx, &(), &mut request).await.unwrap();

    assert_eq!(
        request.messages[0].text(),
        "IN THE TRANSCRIPT",
        "only a placeholder body may be replaced"
    );
}

// ── ArtifactIndexTocMiddleware (issue #6014) ─────────────────────────────────

async fn ctx_with_artifacts(
    entries: &[(&str, &str, &str, u64)],
) -> RunContext<crate::agent::tinyagents::host::OpenHumanRunContext> {
    ctx_with_artifacts_and_config(entries, RunConfig::new("mw-test")).await
}

async fn ctx_with_artifacts_and_config(
    entries: &[(&str, &str, &str, u64)],
    config: RunConfig,
) -> RunContext<crate::agent::tinyagents::host::OpenHumanRunContext> {
    use tinyagents_harness::store::StoreRegistry;
    let index = std::sync::Arc::new(
        crate::agent::harness::tool_result_artifacts::ToolResultArtifactIndexStore::new(),
    );
    for (call_id, tool, path, bytes) in entries {
        let mut fields = serde_json::Map::new();
        fields.insert("tool".to_string(), (*tool).into());
        fields.insert("call_id".to_string(), (*call_id).into());
        fields.insert("artifact_path".to_string(), (*path).into());
        fields.insert(
            "original_bytes".to_string(),
            serde_json::Value::from(*bytes),
        );
        tinyagents_harness::store::Store::put(
            index.as_ref(),
            "tool_results",
            call_id,
            fields.into(),
        )
        .await
        .unwrap();
    }
    let mut registry = StoreRegistry::new();
    registry.register(
        crate::agent::harness::tool_result_artifacts::TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE,
        index,
    );
    RunContext::new(
        config,
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    )
    .with_stores(registry)
}

/// Nothing offloaded → no message. The contents list must not spend context
/// saying that there is nothing to point at.

#[path = "middleware_wrap_up_toc_budget_tests.rs"]
mod budget_tests;
