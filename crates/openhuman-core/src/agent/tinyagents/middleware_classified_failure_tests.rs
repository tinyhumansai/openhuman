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
    // A fetch is scoped by host, never by page or query.
    assert!(summary.contains("example.test"), "{summary}");
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

// ── A public site's HTTP status is not OpenHuman's credential (tinytools#47) ──
//
// The error strings below are the exact `web_fetch` renderings from
// tinyhumansai/tinytools#47 (`http_error_message`).

const FETCH_403: &str =
    "HTTP 403 Forbidden from example.test; the site refused the request. Try another source.";
const FETCH_429: &str = "HTTP 429 Too Many Requests from example.test; the site is rate limiting requests. Retry-After: 30. Try another source, or retry later.";
const FETCH_404: &str = "HTTP 404 Not Found from example.test; the page does not exist at this URL. Check the URL or try another source.";
const FETCH_503: &str = "HTTP 503 Service Unavailable from example.test; the server failed to handle the request. Retry later or try another source.";

/// Run one failing `tool` call with `url` and `error` through the breaker.
async fn fail_call(
    mw: &RepeatedToolFailureMiddleware,
    id: &str,
    tool: &str,
    arguments: serde_json::Value,
    error: &str,
) {
    let mut call = TaToolCall::new(id, tool, arguments);
    mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
    let mut result = failing_result(tool, error);
    mw.after_tool(&mut ctx(), &(), &invocation(id, tool), &mut result)
        .await
        .unwrap();
}

fn fetch_args(url: &str) -> serde_json::Value {
    serde_json::json!({ "url": url })
}

#[tokio::test]
async fn one_blocked_website_does_not_stop_the_run() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    fail_call(
        &mw,
        "blocked-1",
        "web_fetch",
        fetch_args("https://example.test/a"),
        FETCH_403,
    )
    .await;
    assert_eq!(drain_pause_count(&handle), 0);
    assert!(slot.lock().unwrap().is_none());
}

#[tokio::test]
async fn repeated_refusals_from_one_host_stop_the_run_after_the_budget() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    // Different pages of one host: a new path must not reset the count.
    for (id, path) in [("r1", "a"), ("r2", "b")] {
        fail_call(
            &mw,
            id,
            "web_fetch",
            fetch_args(&format!("https://example.test/{path}")),
            FETCH_403,
        )
        .await;
    }
    assert_eq!(drain_pause_count(&handle), 0);
    fail_call(
        &mw,
        "r3",
        "web_fetch",
        fetch_args("https://example.test/c?q=3"),
        FETCH_403,
    )
    .await;
    assert_eq!(drain_pause_count(&handle), 1);
    let summary = slot.lock().unwrap().clone().unwrap();
    assert!(summary.contains("site_refused"), "{summary}");
    assert!(summary.contains("example.test"), "{summary}");
    assert!(!summary.contains("authentication"), "{summary}");
}

#[tokio::test]
async fn refusals_from_different_hosts_are_counted_separately() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    for (id, host) in [
        ("h1", "a.test"),
        ("h2", "b.test"),
        ("h3", "c.test"),
        ("h4", "d.test"),
    ] {
        fail_call(
            &mw,
            id,
            "web_fetch",
            fetch_args(&format!("https://{host}/page")),
            &FETCH_403.replace("example.test", host),
        )
        .await;
    }
    assert_eq!(drain_pause_count(&handle), 0);
}

#[tokio::test]
async fn a_good_fetch_from_a_host_clears_its_refusal_count() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    for id in ["c1", "c2"] {
        fail_call(
            &mw,
            id,
            "web_fetch",
            fetch_args("https://example.test/a"),
            FETCH_403,
        )
        .await;
    }
    let mut call = TaToolCall::new("c-ok", "web_fetch", fetch_args("https://example.test/open"));
    mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
    let mut ok = tool_result(
        "web_fetch",
        "status=200 url=https://example.test/open\nhello",
    );
    mw.after_tool(&mut ctx(), &(), &invocation("c-ok", "web_fetch"), &mut ok)
        .await
        .unwrap();
    for id in ["c3", "c4"] {
        fail_call(
            &mw,
            id,
            "web_fetch",
            fetch_args("https://example.test/b"),
            FETCH_403,
        )
        .await;
    }
    assert_eq!(drain_pause_count(&handle), 0);
}

#[tokio::test]
async fn a_credentialed_endpoint_still_stops_on_the_first_403() {
    // The site-refusal exemption is for `web_fetch` of a public URL. The same
    // wording from a tool that talks to an account-bound API is still a
    // credential failure with no retry budget.
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    fail_call(
        &mw,
        "cred-1",
        "composio_execute",
        serde_json::json!({"endpoint": "github/repos", "account_id": "acct-1"}),
        FETCH_403,
    )
    .await;
    assert_eq!(drain_pause_count(&handle), 1);
    let summary = slot.lock().unwrap().clone().unwrap();
    assert!(summary.contains("authentication"), "{summary}");
}

#[test]
fn fetched_site_statuses_map_to_recovery_budgets() {
    let policy = super::super::repeated_failure::recovery_policy;
    assert_eq!(
        policy("web_fetch", FETCH_403, false),
        Some(("site_refused", 2))
    );
    assert_eq!(
        policy("web_fetch", FETCH_429, false),
        Some(("transient", 2))
    );
    assert_eq!(
        policy("web_fetch", FETCH_503, false),
        Some(("transient", 2))
    );
    // A missing page is an ordinary failure: the exact-repeat guard handles it.
    assert_eq!(policy("web_fetch", FETCH_404, false), None);
    // Bare statuses from web_fetch (no host shape) keep today's meaning.
    assert_eq!(
        policy("web_fetch", "403 Forbidden", false),
        Some(("authentication", 0))
    );
    // The same shape from another tool is not exempt.
    assert_eq!(
        policy("composio_execute", FETCH_403, false),
        Some(("authentication", 0))
    );
}

#[test]
fn a_quoted_response_body_does_not_change_the_fetch_verdict() {
    let policy = super::super::repeated_failure::recovery_policy;
    let not_found = format!("{FETCH_404}\nResponse excerpt: 403 Forbidden unauthorized");
    assert_eq!(policy("web_fetch", &not_found, false), None);
    let refused = format!("{FETCH_403}\nResponse excerpt: service unavailable, timed out");
    assert_eq!(
        policy("web_fetch", &refused, false),
        Some(("site_refused", 2))
    );
}

#[test]
fn web_fetch_failure_scope_is_the_host_not_the_page() {
    let scope = super::super::repeated_failure::failure_scope;
    assert_eq!(
        scope("web_fetch", &fetch_args("https://example.test/a?q=1")),
        scope("web_fetch", &fetch_args("https://example.test/b/c"))
    );
    assert_ne!(
        scope("web_fetch", &fetch_args("https://example.test/a")),
        scope("web_fetch", &fetch_args("https://other.test/a"))
    );
}

#[test]
fn fetched_site_status_reads_only_the_web_fetch_error_shape() {
    let status = super::super::fetched_site::fetched_site_status;
    assert_eq!(status(FETCH_403), Some(403));
    assert_eq!(status(FETCH_429), Some(429));
    assert_eq!(status(FETCH_404), Some(404));
    assert_eq!(status(FETCH_503), Some(503));
    assert_eq!(
        status("  HTTP 403 Forbidden from 127.0.0.1; the site refused it."),
        Some(403)
    );
    // Bare statuses, other tools' wording, success codes, and the shape
    // buried mid-text are not the shape.
    for text in [
        "HTTP 403 Forbidden",
        "HTTP 403",
        "403 Forbidden",
        "Gmail API error: 403 insufficient scopes",
        "Command failed (exit 1)\nHTTP 403 Forbidden from example.test; x",
        "HTTP 2000 Weird from example.test; x",
        "HTTP 200 OK from example.test; x",
        "HTTP 403 Forbidden from ; x",
        "HTTP 403 Forbidden from example.test no semicolon",
    ] {
        assert_eq!(status(text), None, "{text}");
    }
}
