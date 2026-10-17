//! A task that ended without finishing is a failed tool call.
//!
//! Production `tool.browser` spans whose task view said `status.state:
//! "failed"` ("Planning failed…", "the rescuer gave up: … HTTP 504") were
//! returned as successes, so the turn, the breaker and the tool status all
//! read a dead task as done.

use super::*;

fn tool() -> BrowserTool {
    let client = Arc::new(BrowserClient::new(Arc::new(
        crate::config::Config::default(),
    )));
    BrowserTool::new(Arc::new(SecurityPolicy::default()), client, 3)
}

fn view(status: Value, summary: &str) -> TaskView {
    serde_json::from_value(json!({
        "id": "t-9",
        "status": status,
        "summary": summary,
        "progress": 0.0,
        "next": []
    }))
    .unwrap()
}

#[tokio::test]
async fn a_failed_task_is_reported_as_an_error_with_its_reason_and_hint() {
    let failed = view(
        json!({"state": "failed", "step": 2,
               "reason": "the rescuer gave up: rescue model returned HTTP 504",
               "hint": "Try again later or narrow the goal", "recoverable": true}),
        "Could not finish the checkout.",
    );
    let error = tool()
        .report(failed)
        .await
        .expect_err("a failed task must not be a success");
    let text = error.to_string();
    assert!(
        text.starts_with(tinytools::status::TASK_FAILED_MARKER),
        "{text}"
    );
    assert!(text.contains("HTTP 504"), "{text}");
    assert!(text.contains("narrow the goal"), "{text}");
    assert!(text.contains("step 2"), "{text}");
    // The view itself rides along, so the task id stays usable.
    assert!(text.contains("\"t-9\""), "{text}");
}

#[tokio::test]
async fn a_planning_failure_keeps_the_host_hint() {
    let failed = view(
        json!({"state": "failed", "step": null,
               "reason": "browser unavailable: Chrome not found. Checked: /usr/bin",
               "hint": "", "recoverable": true}),
        "Planning failed.",
    );
    let text = tool().report(failed).await.unwrap_err().to_string();
    assert!(
        text.contains(crate::modules::browser::CHROME_NOT_FOUND_HINT),
        "{text}"
    );
    assert!(text.contains("before any step ran"), "{text}");
}

#[tokio::test]
async fn a_task_cancelled_under_it_is_an_error() {
    let cancelled = view(json!({"state": "cancelled"}), "Stopped.");
    let text = tool().report(cancelled).await.unwrap_err().to_string();
    assert!(text.contains("cancelled"), "{text}");
}

#[tokio::test]
async fn running_paused_and_finished_tasks_stay_successes() {
    let tool = tool();
    for status in [
        json!({"state": "running"}),
        json!({"state": "needs_human", "reason": "Solve the captcha"}),
        json!({"state": "needs_input", "fields": []}),
        json!({"state": "done", "answer": "Booked.", "records": {}}),
        json!({"state": "checkpoint", "reason": "payment", "location": "/pay",
               "summary": "At the payment page", "continuable": false}),
    ] {
        let output = tool.report(view(status.clone(), "ok")).await;
        assert!(output.is_ok(), "{status}: {output:?}");
    }
}
