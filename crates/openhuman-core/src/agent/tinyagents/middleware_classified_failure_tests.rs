use super::*;

#[tokio::test]
async fn varied_queries_against_one_forbidden_endpoint_stop_on_first_failure() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let mut call = TaToolCall::new(
        "forbidden-1",
        "web_fetch",
        serde_json::json!({
            "url": "https://example.test/restricted?query=first"
        }),
    );
    mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
    let mut result = failing_result("web_fetch", "403 Forbidden");
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("forbidden-1", "web_fetch"),
        &mut result,
    )
    .await
    .unwrap();
    assert_eq!(drain_pause_count(&handle), 1);
    let summary = slot.lock().unwrap().clone().unwrap();
    assert!(summary.contains("authentication"), "{summary}");
    assert!(summary.contains("example.test/restricted"), "{summary}");
    assert!(!summary.contains("query=first"), "{summary}");
}

#[tokio::test]
async fn schema_repair_gets_one_attempt_even_when_arguments_change() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    for (id, value) in [("schema-1", 1), ("schema-2", 2)] {
        let mut call = TaToolCall::new(
            id,
            "search",
            serde_json::json!({"query": value, "endpoint": "catalog"}),
        );
        mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
        let mut result =
            failing_result("search", "schema validation failed: query must be a string");
        mw.after_tool(&mut ctx(), &(), &invocation(id, "search"), &mut result)
            .await
            .unwrap();
    }
    assert_eq!(drain_pause_count(&handle), 1);
    let summary = slot.lock().unwrap().clone().unwrap();
    assert!(summary.contains("validation"), "{summary}");
    assert!(summary.contains("2 attempt(s)"), "{summary}");
}

#[tokio::test]
async fn missing_desktop_window_allows_one_rediscovery_then_stops() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    for id in ["window-1", "window-2"] {
        let mut call = TaToolCall::new(
            id,
            "tinydesktop_click",
            serde_json::json!({"window_id": 17, "element": id}),
        );
        mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
        let mut result = failing_result(
            "tinydesktop_click",
            r#"{"error":{"code":"WINDOW_NOT_FOUND"}}"#,
        );
        mw.after_tool(
            &mut ctx(),
            &(),
            &invocation(id, "tinydesktop_click"),
            &mut result,
        )
        .await
        .unwrap();
    }
    assert_eq!(drain_pause_count(&handle), 1);
    let summary = slot.lock().unwrap().clone().unwrap();
    assert!(summary.contains("missing_window"), "{summary}");
    assert!(summary.contains("17"), "{summary}");
}

#[tokio::test]
async fn transient_failures_have_two_retries_and_success_clears_only_that_scope() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    for (id, endpoint) in [("a1", "alpha"), ("b1", "beta"), ("a2", "alpha")] {
        let mut call = TaToolCall::new(
            id,
            "web_fetch",
            serde_json::json!({"endpoint": endpoint, "query": id}),
        );
        mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
        let mut result = failing_result("web_fetch", "503 Service Unavailable");
        mw.after_tool(&mut ctx(), &(), &invocation(id, "web_fetch"), &mut result)
            .await
            .unwrap();
    }
    assert_eq!(drain_pause_count(&handle), 0);
    let mut recovered = TaToolCall::new(
        "a-ok",
        "web_fetch",
        serde_json::json!({"endpoint": "alpha"}),
    );
    mw.before_tool(&mut ctx(), &(), &mut recovered)
        .await
        .unwrap();
    let mut ok = tool_result("web_fetch", "ready");
    mw.after_tool(&mut ctx(), &(), &invocation("a-ok", "web_fetch"), &mut ok)
        .await
        .unwrap();
    for id in ["b2", "b3"] {
        let mut call = TaToolCall::new(
            id,
            "web_fetch",
            serde_json::json!({"endpoint": "beta", "query": id}),
        );
        mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
        let mut result = failing_result("web_fetch", "503 Service Unavailable");
        mw.after_tool(&mut ctx(), &(), &invocation(id, "web_fetch"), &mut result)
            .await
            .unwrap();
    }
    assert_eq!(drain_pause_count(&handle), 1);
    let summary = slot.lock().unwrap().clone().unwrap();
    assert!(summary.contains("3 attempt(s)"), "{summary}");
    assert!(summary.contains("beta"), "{summary}");
}

#[test]
fn uncertain_timeout_requires_reconciliation() {
    assert_eq!(
        super::super::repeated_failure::recovery_policy("gmail_send", "timed out", false),
        Some(("uncertain_side_effect", 0))
    );
    assert_eq!(
        super::super::repeated_failure::recovery_policy("web_fetch", "timed out", false),
        Some(("transient", 2))
    );
    // A killed local command is inspectable, so it gets one recovery attempt.
    assert_eq!(
        super::super::repeated_failure::recovery_policy(
            "shell",
            "Command timed out after 60s and was killed",
            false
        ),
        Some(("uncertain_side_effect", 1))
    );
}

/// Regression: one `whois` loop hitting the 60s shell timeout halted the whole
/// turn, discarding every earlier result. The first timeout must steer the
/// model to reconcile and narrow the command; only a second one halts.
#[tokio::test]
async fn shell_timeout_nudges_once_then_halts_on_second() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let run = |id: &'static str, command: &'static str| {
        let mw = &mw;
        async move {
            let mut call = TaToolCall::new(id, "shell", serde_json::json!({ "command": command }));
            mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
            let mut result = failing_result("shell", "Command timed out after 60s and was killed");
            mw.after_tool(&mut ctx(), &(), &invocation(id, "shell"), &mut result)
                .await
                .unwrap();
        }
    };

    run("sh-1", "for d in a.io b.io c.io; do whois $d; done").await;
    assert_eq!(drain_pause_count(&handle), 0, "first timeout must not halt");
    assert!(slot.lock().unwrap().is_none());
    let nudges = mw.take_pending_nudges();
    assert_eq!(nudges.len(), 1, "{nudges:?}");
    assert!(nudges[0].contains("timed out"), "{nudges:?}");
    assert!(nudges[0].contains("smaller, bounded"), "{nudges:?}");

    run("sh-2", "whois a.io").await;
    assert_eq!(drain_pause_count(&handle), 1, "second timeout halts");
    let summary = slot.lock().unwrap().clone().unwrap();
    assert!(summary.contains("uncertain_side_effect"), "{summary}");
    assert!(
        summary.contains("reconcile its external state"),
        "{summary}"
    );
}

/// A shell success in between clears the ledger, so a later, unrelated timeout
/// gets its own recovery attempt instead of halting.
#[tokio::test]
async fn shell_success_resets_the_timeout_budget() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    for (id, ok) in [("t-1", false), ("t-ok", true), ("t-2", false)] {
        let mut call = TaToolCall::new(id, "shell", serde_json::json!({ "command": id }));
        mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
        let mut result = if ok {
            tool_result("shell", "done")
        } else {
            failing_result("shell", "Command timed out after 60s and was killed")
        };
        mw.after_tool(&mut ctx(), &(), &invocation(id, "shell"), &mut result)
            .await
            .unwrap();
    }
    assert_eq!(drain_pause_count(&handle), 0);
    assert!(slot.lock().unwrap().is_none());
}

#[test]
fn structured_status_precedes_ambiguous_error_prose() {
    assert_eq!(
        super::super::repeated_failure::recovery_policy(
            "search",
            r#"{"status_code":403,"message":"try again later"}"#,
            false
        ),
        Some(("permission", 0))
    );
    assert_eq!(
        super::super::repeated_failure::recovery_policy(
            "search",
            r#"{"error":{"code":"INVALID_ARGUMENT"}}"#,
            false
        ),
        Some(("validation", 1))
    );
    assert_eq!(
        super::super::repeated_failure::recovery_policy(
            "tinydesktop_click",
            r#"{"error":{"code":"WINDOW_NOT_FOUND"}}"#,
            false
        ),
        Some(("missing_window", 1))
    );
}

#[test]
fn failure_scope_keeps_resource_and_permission_context_but_ignores_query() {
    let first = super::super::repeated_failure::failure_scope(
        "web_fetch",
        &serde_json::json!({"account_id": "team-a", "url": "https://example.test/restricted?q=one"}),
    );
    let second = super::super::repeated_failure::failure_scope(
        "web_fetch",
        &serde_json::json!({"account_id": "team-a", "url": "https://example.test/restricted?q=two"}),
    );
    let other = super::super::repeated_failure::failure_scope(
        "web_fetch",
        &serde_json::json!({"account_id": "team-b", "url": "https://example.test/restricted?q=two"}),
    );
    assert_eq!(first, second);
    assert_ne!(first, other);
    assert!(!first.contains("q="));
    assert_ne!(
        super::super::repeated_failure::failure_scope(
            "desktop_click",
            &serde_json::json!({"app":"Browser", "window_id": 1})
        ),
        super::super::repeated_failure::failure_scope(
            "desktop_click",
            &serde_json::json!({"app":"Browser", "window_id": 2})
        ),
    );
}

#[tokio::test]
async fn legitimate_wait_polling_does_not_consume_transient_budget() {
    let handle = SteeringHandle::allow_all();
    let mw = RepeatedToolFailureMiddleware::new(
        handle.clone(),
        3,
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    );
    for id in 0..5 {
        let mut result = failing_result("wait_subagent", "timed out waiting for child");
        mw.after_tool(
            &mut ctx(),
            &(),
            &invocation(format!("wait-{id}"), "wait_subagent"),
            &mut result,
        )
        .await
        .unwrap();
    }
    assert_eq!(drain_pause_count(&handle), 0);
}

/// An unknown-tool answer echoes the attempted name and every valid tool name.
/// None of those names is the failure: a guess named `forbidden_tool`, or a
/// belt with a tool whose name carries `unauthorized`, used to classify as
/// `authentication` (zero retries) and halt the run on the first wrong guess.
#[test]
fn an_unknown_tool_is_a_correctable_call_not_a_blocker() {
    for error in [
        "unknown tool `forbidden_tool` (arguments: {}); valid tools: [file_read]",
        "unknown tool `ranges` (arguments: {}); valid tools: [GITHUB_LIST_UNAUTHORIZED_USERS, file_write]",
        // Current tinyagents corrective: close matches + a `tool_search` pointer.
        "unknown tool `forbidden_tool`: no tool with that name is available to you, and calling it again will fail the same way. Closest available: `forbidden_list_unauthorized`. To find the right tool, call `tool_search` with what you want to do in plain words, then call a tool it returns.",
    ] {
        assert_eq!(
            super::super::repeated_failure::recovery_policy("forbidden_tool", error, false),
            Some(("validation", 1)),
            "{error}"
        );
    }
    // A genuine credential failure is still one.
    assert_eq!(
        super::super::repeated_failure::recovery_policy("gmail_send", "401 Unauthorized", false),
        Some(("authentication", 0))
    );
}

#[test]
fn a_mistyped_file_path_is_a_correctable_call_not_a_missing_program() {
    let error = "Failed to resolve path 'mailbox/2026-09-18-flight.txt': No such file or directory (os error 2)";
    for tool in ["file_read", "file_write", "apply_patch"] {
        assert_eq!(
            super::super::repeated_failure::recovery_policy(tool, error, false),
            Some(("not_found", 1)),
            "{tool}"
        );
    }
    // A shell that cannot find a program is still unsupported.
    assert_eq!(
        super::super::repeated_failure::recovery_policy(
            "shell",
            "bash: jq: command not found",
            false
        ),
        Some(("unsupported", 0))
    );
}

/// Queue one validation/no-progress-style nudge the way a first shell timeout
/// does, and return the middleware holding it.
async fn middleware_with_one_pending_nudge() -> RepeatedToolFailureMiddleware {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle, 3, slot);
    let mut call = TaToolCall::new(
        "sh-1",
        "shell",
        serde_json::json!({ "command": "whois a.io" }),
    );
    mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
    let mut result = failing_result("shell", "Command timed out after 60s and was killed");
    mw.after_tool(&mut ctx(), &(), &invocation("sh-1", "shell"), &mut result)
        .await
        .unwrap();
    mw
}

fn nudge_request(leading: &TaMessage) -> ModelRequest {
    ModelRequest {
        messages: vec![
            leading.clone(),
            TaMessage::user("look up a.io"),
            TaMessage::tool("sh-1", "Command timed out after 60s and was killed"),
        ],
        ..Default::default()
    }
}

fn system_messages(request: &ModelRequest) -> usize {
    request
        .messages
        .iter()
        .filter(|message| matches!(message, TaMessage::System(_)))
        .count()
}

/// #6962: DeepSeek moves every system turn to the prompt head, so a nudge sent
/// as a new system message reset its prompt cache to the static prefix. For a
/// model whose profile hoists system messages the nudge rides the tail tool
/// result instead, and the leading system message stays byte-identical.
#[tokio::test]
async fn nudges_for_a_hoisting_model_add_no_system_message() {
    let mw = middleware_with_one_pending_nudge().await;
    let mut run_ctx = ctx();
    run_ctx.model_profile = Some(tinyinference_llm::model::ModelProfile {
        hoists_system_messages: true,
        mid_conversation_system_messages: true,
        ..Default::default()
    });
    let leading = TaMessage::system("persona");
    let mut request = nudge_request(&leading);

    mw.nudge_injector()
        .before_model(&mut run_ctx, &(), &mut request)
        .await
        .unwrap();

    assert_eq!(request.messages.len(), 3, "no message added");
    assert_eq!(
        request.messages[0], leading,
        "leading system message untouched"
    );
    assert_eq!(system_messages(&request), 1, "no new system message");
    let tail = request.messages.last().unwrap().text();
    assert!(tail.starts_with("Command timed out"), "{tail}");
    assert!(tail.contains("smaller, bounded"), "nudge delivered: {tail}");
}

/// Other models keep the tail system message the nudge always used.
#[tokio::test]
async fn nudges_for_other_models_stay_tail_system_messages() {
    let mw = middleware_with_one_pending_nudge().await;
    let leading = TaMessage::system("persona");
    let mut request = nudge_request(&leading);

    mw.nudge_injector()
        .before_model(&mut ctx(), &(), &mut request)
        .await
        .unwrap();

    assert_eq!(request.messages.len(), 4);
    assert_eq!(request.messages[0], leading);
    assert!(matches!(
        request.messages.last(),
        Some(TaMessage::System(_))
    ));
    assert!(request
        .messages
        .last()
        .unwrap()
        .text()
        .contains("smaller, bounded"));
}
