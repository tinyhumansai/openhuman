use super::*;

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
    assert!(
        msg.contains("do not re-run"),
        "must discourage re-running: {msg}"
    );
    // And must not also say the opposite. The loop this fixes came from an
    // affirmative instruction, so a message carrying both would read as
    // permission to re-run while the assertion above still passed. Every
    // mention of re-running has to be the negated one.
    let mut cursor = 0;
    while let Some(found) = msg[cursor..].find("re-run") {
        let at = cursor + found;
        assert!(
            msg[..at].trim_end().ends_with("do not"),
            "every mention of re-running must be negated, found a bare one at {at}: {msg}"
        );
        cursor = at + "re-run".len();
    }
    assert!(
        msg.contains("compacted summary"),
        "must point at the summary: {msg}"
    );
}

#[tokio::test]
async fn retrieve_messages_name_only_the_tool_the_model_can_call() {
    // `tokenjuice_retrieve` / `tinyjuice_retrieve` are dispatch aliases for old
    // transcripts, not tools a model is offered. A message naming them sends the
    // model after a tool it cannot see ("unknown tool").
    let missing = TokenjuiceRetrieveTool::new()
        .execute(json!({}))
        .await
        .unwrap()
        .output();
    for msg in [missing, miss_message("deadbeefcafe")] {
        assert!(msg.contains(tinyjuice_bus::tools::RETRIEVE), "{msg}");
        assert!(!msg.contains("tokenjuice_retrieve"), "{msg}");
        assert!(!msg.contains("tinyjuice_retrieve"), "{msg}");
    }
}
