use super::*;
#[tokio::test]
#[ignore = "requires a built TinyJuice module"]
async fn retrieves_offloaded_original() {
    let original = "ORIGINAL TOKENJUICE PAYLOAD ".repeat(20);
    let hash = "module-fixture";
    let tool = TokenjuiceRetrieveTool::new();
    let res = tool.execute(json!({ "token": hash })).await.unwrap();
    assert!(!res.is_error);
    assert_eq!(res.output(), original);
}

#[tokio::test]
#[ignore = "requires a built TinyJuice module"]
async fn retrieves_line_range() {
    let _original = "r0\nr1\nr2\nr3\nr4";
    let hash = "module-fixture";
    let tool = TokenjuiceRetrieveTool::new();
    let res = tool
        .execute(json!({ "token": hash, "range": { "start": 1, "end": 3, "unit": "lines" } }))
        .await
        .unwrap();
    assert!(!res.is_error);
    assert_eq!(res.output(), "r1\nr2");
}

#[tokio::test]
async fn missing_token_is_error() {
    let tool = TokenjuiceRetrieveTool::new();
    let res = tool
        .execute(json!({ "token": "deadbeefcafe" }))
        .await
        .unwrap();
    assert!(res.is_error);
    let res2 = tool.execute(json!({})).await.unwrap();
    assert!(res2.is_error);
}

#[test]
fn miss_message_does_not_instruct_a_blind_re_run() {
    // See `miss_message` — "re-run the tool" here is what turned a single
    // eviction into an unbounded compact→retrieve→re-run loop.
    let msg = miss_message("deadbeefcafe").to_lowercase();
    assert!(!msg.contains("re-run the same tool") || msg.contains("do not re-run"));
    assert!(
        msg.contains("do not re-run"),
        "must discourage re-running: {msg}"
    );
    assert!(
        msg.contains("compacted summary"),
        "must point at the summary: {msg}"
    );
}
