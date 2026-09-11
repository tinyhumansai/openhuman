use super::*;

#[tokio::test]
async fn current_window_returns_none_outside_scope() {
    assert_eq!(current_task_recency_window(), None);
}

#[tokio::test]
async fn with_window_installs_value() {
    let observed = with_task_recency_window(Duration::from_secs(86_400), async {
        current_task_recency_window()
    })
    .await;
    assert_eq!(observed, Some(Duration::from_secs(86_400)));
}

#[tokio::test]
async fn with_window_does_not_leak_across_scopes() {
    with_task_recency_window(Duration::from_secs(60), async {
        assert_eq!(current_task_recency_window(), Some(Duration::from_secs(60)));
    })
    .await;
    assert_eq!(current_task_recency_window(), None);
}

#[tokio::test]
async fn nested_scope_overrides_outer() {
    with_task_recency_window(Duration::from_secs(60), async {
        assert_eq!(current_task_recency_window(), Some(Duration::from_secs(60)));
        with_task_recency_window(Duration::from_secs(120), async {
            assert_eq!(
                current_task_recency_window(),
                Some(Duration::from_secs(120))
            );
        })
        .await;
        assert_eq!(current_task_recency_window(), Some(Duration::from_secs(60)));
    })
    .await;
}
