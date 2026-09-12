use super::*;

#[tokio::test]
async fn current_sandbox_mode_returns_none_outside_scope() {
    assert_eq!(current_sandbox_mode(), None);
}

#[tokio::test]
async fn with_current_sandbox_mode_installs_read_only() {
    let observed =
        with_current_sandbox_mode(SandboxMode::ReadOnly, async { current_sandbox_mode() }).await;
    assert_eq!(observed, Some(SandboxMode::ReadOnly));
}

#[tokio::test]
async fn with_current_sandbox_mode_does_not_leak_across_scopes() {
    with_current_sandbox_mode(SandboxMode::ReadOnly, async {
        assert_eq!(current_sandbox_mode(), Some(SandboxMode::ReadOnly));
    })
    .await;
    assert_eq!(current_sandbox_mode(), None);
}

#[tokio::test]
async fn nested_scope_overrides_outer() {
    with_current_sandbox_mode(SandboxMode::ReadOnly, async {
        assert_eq!(current_sandbox_mode(), Some(SandboxMode::ReadOnly));
        with_current_sandbox_mode(SandboxMode::Sandboxed, async {
            assert_eq!(current_sandbox_mode(), Some(SandboxMode::Sandboxed));
        })
        .await;
        assert_eq!(current_sandbox_mode(), Some(SandboxMode::ReadOnly));
    })
    .await;
}
