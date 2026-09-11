use super::*;

#[tokio::test]
async fn set_get_complete_clear_flow() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Empty to start.
    let got = get(dir, "t").await.unwrap();
    assert!(got.value.goal.is_none());

    // Set.
    let set_out = set(dir, "t", "ship it", Some(1000)).await.unwrap();
    let goal = set_out.value.goal.unwrap();
    assert_eq!(goal.objective, "ship it");

    // Complete.
    let done = complete(dir, "t").await.unwrap();
    assert_eq!(done.value.goal.unwrap().status.as_str(), "complete");

    // Clear.
    let cleared = clear(dir, "t").await.unwrap();
    assert!(cleared.value.removed);
    assert!(get(dir, "t").await.unwrap().value.goal.is_none());
}
