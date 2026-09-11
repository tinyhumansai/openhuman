use super::*;

#[tokio::test]
async fn status_response_wraps_snapshot_key() {
    let resp = handle_status(Map::new()).await.unwrap();
    assert!(resp.get("snapshot").is_some());
    let overall = resp["snapshot"]["overall"].as_str().unwrap();
    // Idle before any run, or a later state if a boot run already executed
    // in this process — both are valid, we only assert the shape.
    assert!(["idle", "running", "done", "failed"].contains(&overall));
    assert!(resp["snapshot"]["steps"].is_array());
}
