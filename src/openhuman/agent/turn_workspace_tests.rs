use super::*;

#[tokio::test]
async fn scope_sets_and_clears_the_root() {
    assert!(current().is_none(), "baseline outside any scope");
    let observed = with_workspace(PathBuf::from("/work/checkout"), async { current() }).await;
    assert_eq!(observed, Some(PathBuf::from("/work/checkout")));
    assert!(current().is_none(), "root must not leak past the scope");
}

/// A detached sub-agent must keep working in the tree its parent turn was
/// bound to; a bare `tokio::spawn` drops the root, which is what put its
/// writes back outside the sandbox.
#[tokio::test]
async fn propagate_carries_the_root_across_a_spawn() {
    let observed = with_workspace(PathBuf::from("/work/checkout"), async {
        tokio::spawn(propagate(async { current() }))
            .await
            .expect("spawned task panicked")
    })
    .await;
    assert_eq!(observed, Some(PathBuf::from("/work/checkout")));
}

/// Without the wrapper the same spawn loses the root — pinned so the fix
/// cannot be silently undone.
#[tokio::test]
async fn a_bare_spawn_loses_the_root() {
    let observed = with_workspace(PathBuf::from("/work/checkout"), async {
        tokio::spawn(async { current() })
            .await
            .expect("spawned task panicked")
    })
    .await;
    assert!(observed.is_none());
}

#[tokio::test]
async fn nested_scope_overrides_the_outer_root() {
    with_workspace(PathBuf::from("/outer"), async {
        assert_eq!(current(), Some(PathBuf::from("/outer")));
        with_workspace(PathBuf::from("/inner"), async {
            assert_eq!(current(), Some(PathBuf::from("/inner")));
        })
        .await;
        assert_eq!(current(), Some(PathBuf::from("/outer")));
    })
    .await;
}
