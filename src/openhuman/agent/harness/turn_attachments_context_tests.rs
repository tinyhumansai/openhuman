use super::*;

#[tokio::test]
async fn empty_outside_scope() {
    assert!(current_turn_image_placeholders().is_empty());
}

#[tokio::test]
async fn installs_and_reads_back() {
    let observed = with_current_turn_image_placeholders(
        vec!["[Image: image #att:abc123]".to_string()],
        async { current_turn_image_placeholders() },
    )
    .await;
    assert_eq!(observed, vec!["[Image: image #att:abc123]".to_string()]);
}

#[tokio::test]
async fn does_not_leak_across_scopes() {
    with_current_turn_image_placeholders(vec!["x".to_string()], async {
        assert_eq!(current_turn_image_placeholders().len(), 1);
    })
    .await;
    assert!(current_turn_image_placeholders().is_empty());
}
