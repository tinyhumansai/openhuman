use super::*;

#[tokio::test]
async fn current_spawn_depth_defaults_to_zero() {
    assert_eq!(current_spawn_depth(), 0);
}

#[tokio::test]
async fn with_spawn_depth_scopes_value_to_future() {
    let observed = with_spawn_depth(2, async { current_spawn_depth() }).await;
    assert_eq!(observed, 2);
    assert_eq!(current_spawn_depth(), 0);
}

#[tokio::test]
async fn nested_spawn_depth_scope_restores_outer_value() {
    with_spawn_depth(1, async {
        assert_eq!(current_spawn_depth(), 1);
        with_spawn_depth(2, async {
            assert_eq!(current_spawn_depth(), 2);
        })
        .await;
        assert_eq!(current_spawn_depth(), 1);
    })
    .await;
}
