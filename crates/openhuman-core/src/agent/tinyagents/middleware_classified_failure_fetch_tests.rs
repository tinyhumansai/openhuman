use super::*;
use crate::agent::tinyagents::middleware::repeated_failure::RepeatedToolFailureMiddleware;

const FETCH_403: &str =
    "HTTP 403 Forbidden from example.test; the site refused the request. Try another source.";
const FETCH_429: &str = "HTTP 429 Too Many Requests from example.test; the site is rate limiting requests. Retry-After: 30. Try another source, or retry later.";
const FETCH_404: &str = "HTTP 404 Not Found from example.test; the page does not exist at this URL. Check the URL or try another source.";
const FETCH_503: &str = "HTTP 503 Service Unavailable from example.test; the server failed to handle the request. Retry later or try another source.";

// Fetched-site refusals: per-host budgets, steering off a credentialed
// endpoint, and the status-to-budget mapping.

/// Run one failing `tool` call with `url` and `error` through the breaker.
async fn fail_call(
    mw: &RepeatedToolFailureMiddleware,
    context: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    id: &str,
    tool: &str,
    arguments: serde_json::Value,
    error: &str,
) {
    let mut call = TaToolCall::new(id, tool, arguments);
    mw.before_tool(context, &(), &mut call).await.unwrap();
    let mut result = failing_result(tool, error);
    mw.after_tool(context, &(), &invocation(id, tool), &mut result)
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
    let mut context = ctx();
    fail_call(
        &mw,
        &mut context,
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
    let mut context = ctx();
    // Different pages of one host: a new path must not reset the count.
    for (id, path) in [("r1", "a"), ("r2", "b")] {
        fail_call(
            &mw,
            &mut context,
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
        &mut context,
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
    let mut context = ctx();
    for (id, host) in [
        ("h1", "a.test"),
        ("h2", "b.test"),
        ("h3", "c.test"),
        ("h4", "d.test"),
    ] {
        fail_call(
            &mw,
            &mut context,
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
    let mut context = ctx();
    for id in ["c1", "c2"] {
        fail_call(
            &mw,
            &mut context,
            id,
            "web_fetch",
            fetch_args("https://example.test/a"),
            FETCH_403,
        )
        .await;
    }
    let mut call = TaToolCall::new("c-ok", "web_fetch", fetch_args("https://example.test/open"));
    mw.before_tool(&mut context, &(), &mut call).await.unwrap();
    let mut ok = tool_result(
        "web_fetch",
        "status=200 url=https://example.test/open\nhello",
    );
    mw.after_tool(&mut context, &(), &invocation("c-ok", "web_fetch"), &mut ok)
        .await
        .unwrap();
    for id in ["c3", "c4"] {
        fail_call(
            &mw,
            &mut context,
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
async fn a_credentialed_endpoint_is_steered_off_then_stopped() {
    // The site-refusal exemption is for `web_fetch` of a public URL. The same
    // wording from a tool that talks to an account-bound API is a credential
    // failure: the connector is not available in this session. That is a tool
    // to stop using, not a reason to end the run, so the first refusal steers
    // the model off it and only a repeat of the same operation stops the run.
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let mut context = ctx();
    fail_call(
        &mw,
        &mut context,
        "cred-1",
        "composio_execute",
        serde_json::json!({"endpoint": "github/repos", "account_id": "acct-1"}),
        FETCH_403,
    )
    .await;
    assert_eq!(
        drain_pause_count(&handle),
        0,
        "first refusal steers, not stops"
    );
    assert!(slot.lock().unwrap().is_none());
    let nudges = drain_nudge_messages(&mw, &mut context).await;
    assert_eq!(nudges.len(), 1, "{nudges:?}");
    assert!(
        nudges[0].contains("`composio_execute` tool cannot be used"),
        "{nudges:?}"
    );
    fail_call(
        &mw,
        &mut context,
        "cred-2",
        "composio_execute",
        serde_json::json!({"endpoint": "github/repos", "account_id": "acct-1"}),
        FETCH_403,
    )
    .await;
    assert_eq!(
        drain_pause_count(&handle),
        1,
        "the same refused operation again stops"
    );
    let summary = slot.lock().unwrap().clone().unwrap();
    assert!(summary.contains("service_refused"), "{summary}");
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
    // The same shape from a connector is not a site refusal either: it is the
    // connector's own credential failure, which steers the model off it once.
    assert_eq!(
        policy("composio_execute", FETCH_403, false),
        Some(("service_refused", 1))
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
