use super::*;

fn test_config() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    (dir, config)
}

/// Regression (TAURI-RUST-HMW / #4337): `with_root_parent` must hand a real
/// parent to the wrapped closure so a background orchestration surface (cron
/// job, workflow engine, team runtime) can spawn sub-agents without hitting
/// `NoParentContext`. Proven by observing the parent the closure receives.
#[tokio::test]
async fn with_root_parent_hands_a_root_parent_to_the_closure() {
    let (_dir, config) = test_config();
    let observed = with_root_parent(
        &crate::core::runtime::DispatchCtx::ambient(),
        None,
        &config,
        "cron",
        "cron",
        "cron",
        |parent| async move { parent.agent_definition_id },
    )
    .await
    .expect("root parent builds from config");
    assert_eq!(
        observed, "cron",
        "the closure must receive the root parent built from config"
    );
}

/// When a parent is passed explicitly, `with_root_parent` uses it instead of
/// building a second root, so a test can drive a surface under a mock parent
/// (and thus a mock provider) hermetically.
#[tokio::test]
async fn with_root_parent_uses_an_explicit_parent_override() {
    let (_dir, config) = test_config();
    let outer = build_root_parent(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "outer",
        "outer",
        "outer",
    )
    .await
    .expect("build explicit parent");
    let observed = with_root_parent(
        &crate::core::runtime::DispatchCtx::ambient(),
        Some(outer),
        &config,
        "inner",
        "inner",
        "inner",
        |parent| async move { parent.agent_definition_id },
    )
    .await
    .expect("uses the override, no build error");
    assert_eq!(
        observed, "outer",
        "with_root_parent must use the explicit parent, not build a new 'inner' root"
    );
}

/// The parent a surface spawns under is the value the closure received, not
/// something read from the executing task: two concurrent surfaces on one
/// runtime each see their own root.
#[tokio::test]
async fn concurrent_root_parents_do_not_cross() {
    let (_dir, config) = test_config();
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let (a, b) = tokio::join!(
        with_root_parent(
            &ctx,
            None,
            &config,
            "surface_a",
            "a",
            "a",
            |parent| async move {
                tokio::task::yield_now().await;
                parent.agent_definition_id
            }
        ),
        with_root_parent(
            &ctx,
            None,
            &config,
            "surface_b",
            "b",
            "b",
            |parent| async move {
                tokio::task::yield_now().await;
                parent.agent_definition_id
            }
        ),
    );
    assert_eq!(a.expect("a"), "surface_a");
    assert_eq!(b.expect("b"), "surface_b");
}
