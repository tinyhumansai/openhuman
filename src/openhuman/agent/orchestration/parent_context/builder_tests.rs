use super::*;
use crate::openhuman::agent::harness::fork_context::current_parent;

fn test_config() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    (dir, config)
}

/// Baseline for the bug: with no enclosing agent turn there is no ambient
/// parent — exactly the state the subconscious tick spawned `context_scout`
/// in (TAURI-RUST-HMW / #4337), which made `run_subagent` return
/// `NoParentContext`.
#[tokio::test]
async fn no_ambient_parent_outside_with_root_parent() {
    assert!(
        current_parent().is_none(),
        "no parent context should be installed by default"
    );
}

/// Regression (TAURI-RUST-HMW / #4337): `with_root_parent` must install a
/// real parent for the wrapped future so a background orchestration surface
/// (subconscious tick, workflow engine, team runtime) can spawn sub-agents
/// without hitting `NoParentContext`. Proven by observing the installed
/// parent from inside the future.
#[tokio::test]
async fn with_root_parent_installs_parent_for_inner_future() {
    let (_dir, config) = test_config();
    let observed = with_root_parent(
        &config,
        "subconscious",
        "subconscious",
        "subconscious",
        async { current_parent().map(|p| p.agent_definition_id) },
    )
    .await
    .expect("root parent builds from config");
    assert_eq!(
        observed.as_deref(),
        Some("subconscious"),
        "inner future must observe the installed root parent"
    );
}

/// When a parent is already installed, `with_root_parent` reuses it instead
/// of building a second root — so a surface nested in a turn (or a test
/// driving it under a mock parent) runs under the ambient context.
#[tokio::test]
async fn with_root_parent_reuses_ambient_parent() {
    let (_dir, config) = test_config();
    let outer = build_root_parent(&config, "outer", "outer", "outer")
        .await
        .expect("build ambient parent");
    let observed = with_parent_context(outer, async {
        with_root_parent(&config, "inner", "inner", "inner", async {
            current_parent().map(|p| p.agent_definition_id)
        })
        .await
        .expect("reuses ambient, no build error")
    })
    .await;
    assert_eq!(
        observed.as_deref(),
        Some("outer"),
        "with_root_parent must reuse the ambient parent, not build a new 'inner' root"
    );
}
