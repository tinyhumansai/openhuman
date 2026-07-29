use super::*;
#[tokio::test]
#[ignore = "requires a built TinyJuice module"]
async fn retrieves_offloaded_original() {
    let original = "ORIGINAL PAYLOAD ".repeat(20);
    let hash = "module-fixture";
    let tool = RetrieveToolOutputTool::new();
    let res = tool.execute(json!({ "hash": hash })).await.unwrap();
    assert!(!res.is_error);
    assert_eq!(res.output(), original);
}

#[tokio::test]
async fn missing_hash_is_error() {
    let tool = RetrieveToolOutputTool::new();
    let res = tool
        .execute(json!({ "hash": "deadbeefcafe" }))
        .await
        .unwrap();
    assert!(res.is_error);
    let res2 = tool.execute(json!({})).await.unwrap();
    assert!(res2.is_error);
}

#[tokio::test]
async fn a_cache_miss_does_not_tell_the_model_to_re_run() {
    // The loop this guards against: a miss that says "re-run the tool" makes
    // the agent regenerate the same oversized result, which is compacted and
    // evicted again — an unbounded compact→retrieve→re-run loop.
    let tool = RetrieveToolOutputTool::new();
    let msg = tool
        .execute(json!({ "hash": "deadbeefcafe" }))
        .await
        .unwrap()
        .output();
    assert!(
        !msg.to_lowercase().contains("re-run the tool"),
        "a miss must not instruct a blind re-run: {msg}"
    );
    assert!(
        msg.to_lowercase().contains("do not re-run"),
        "a miss must explicitly discourage re-running: {msg}"
    );
    assert!(
        msg.contains("compacted summary"),
        "a miss must point the model at the summary it already has: {msg}"
    );
}
