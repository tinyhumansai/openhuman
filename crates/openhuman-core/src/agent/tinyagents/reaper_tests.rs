use super::*;

use tinyagents_harness::events::HarnessRunStatus;
use tinyagents_harness::ids::{ComponentId, ExecutionStatus, HarnessPhase};
use tinyagents_harness::observability::{
    mint_run_id, DriverStatusStore, HarnessStatusStore, ORPHAN_REAP_REASON,
};

/// Build a fresh status in a non-terminal state and persist it, started a
/// minute ago like a run a previous process left behind (the spawned sweep
/// only reaps runs that started before it, in whole seconds).
async fn seed_status(store: &DriverStatusStore, status_kind: ExecutionStatus) -> String {
    let run_id = mint_run_id();
    let mut status =
        HarnessRunStatus::new(run_id.clone(), ComponentId::new("mock-model".to_string()));
    match status_kind {
        ExecutionStatus::Running => status.mark_running(HarnessPhase::Model),
        other => panic!("seed_status only seeds Running here, got {other:?}"),
    }
    status.started_at = SystemTime::now() - Duration::from_secs(60);
    store.put_status(status).await.unwrap();
    run_id.as_str().to_string()
}

/// The workspace adapter reaps a seeded orphan and a missing workspace reaps
/// nothing (the sweep logic itself is tested upstream).
#[tokio::test]
async fn workspace_adapter_reaps_and_empty_workspace_is_a_noop() {
    let tmp = std::env::temp_dir().join(format!("oh-reaper-{}", uuid::Uuid::new_v4()));
    assert_eq!(reap_orphaned_runs(&tmp).await, 0);
    let store = crate::storage::sessions::status_store_for(&tmp).expect("status store");
    let run = seed_status(&store, ExecutionStatus::Running).await;
    assert_eq!(reap_orphaned_runs(&tmp).await, 1);
    let status = store.get_status(&run).await.unwrap().expect("present");
    assert_eq!(status.status, ExecutionStatus::Cancelled);
    assert_eq!(status.error.as_deref(), Some(ORPHAN_REAP_REASON));
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A shared backend never reaps: the run may belong to another replica.
#[tokio::test]
async fn a_shared_backend_leaves_running_work_alone() {
    let tmp = std::env::temp_dir().join(format!("oh-reaper-{}", uuid::Uuid::new_v4()));
    let store = crate::storage::sessions::status_store_for(&tmp).expect("status store");
    let run = seed_status(&store, ExecutionStatus::Running).await;
    assert_eq!(reap_unless_shared(&tmp, true, None).await, 0);
    let status = store.get_status(&run).await.unwrap().expect("present");
    assert_eq!(status.status, ExecutionStatus::Running);
    assert_eq!(reap_unless_shared(&tmp, false, None).await, 1);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A run that started after the sweep's cutoff is this process's own work, not
/// an orphan: the sweep must leave it running even though it is still active.
#[tokio::test]
async fn a_run_started_after_the_cutoff_is_not_reaped() {
    let tmp = std::env::temp_dir().join(format!("oh-reaper-{}", uuid::Uuid::new_v4()));
    let store = crate::storage::sessions::status_store_for(&tmp).expect("status store");
    let orphan = seed_status(&store, ExecutionStatus::Running).await;
    let cutoff = SystemTime::now() + Duration::from_secs(60);
    let mut live = HarnessRunStatus::new(mint_run_id(), ComponentId::new("mock-model".to_string()));
    live.mark_running(HarnessPhase::Model);
    live.started_at = cutoff + Duration::from_secs(5);
    let live_id = live.run_id.as_str().to_string();
    store.put_status(live).await.unwrap();

    assert_eq!(reap_orphaned_runs_before(&tmp, Some(cutoff)).await, 1);
    let orphan = store.get_status(&orphan).await.unwrap().expect("present");
    assert_eq!(orphan.status, ExecutionStatus::Cancelled);
    let live = store.get_status(&live_id).await.unwrap().expect("present");
    assert_eq!(live.status, ExecutionStatus::Running);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The cutoff is rounded down to the whole second `started_at` is stored in.
#[test]
fn the_cutoff_is_rounded_down_to_whole_seconds() {
    let at = SystemTime::UNIX_EPOCH + Duration::from_millis(1_700_000_000_750);
    let store = StartedBefore::new((), at);
    assert_eq!(
        store.cutoff,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    );
}

/// The sweep has to happen on the path a **build-only** embedder takes, and
/// `build()` must not wait for it.
///
/// `CoreRuntime::invoke` dispatches `openhuman.agent_runs_active` the moment
/// `build()` returns, with no transport and no background services in
/// between, so a sweep that lived with the other boot-once jobs (which run
/// from `serve()`) would leave that caller reading the previous process's
/// graveyard. This pins the sweep to the build path with every optional
/// service off. The child holds the spawned sweep (`hold_startup_sweeps_for_test`) and checks the
/// orphan is still active once `build()` has returned, which proves boot did
/// not wait for the sweep; it then releases it and waits for it to finish.
///
/// The build itself runs in a **child process** (this same test binary,
/// re-invoked with `--exact` on this test and `REAPER_BOOT_CHILD` set).
/// `CoreBuilder::build` → `CoreContext::init` → `bootstrap_core_runtime`
/// installs process-wide `OnceLock` singletons that can never be unset — the
/// default `CoreContext` (here with `DomainSet::harness()` and a throw-away
/// workspace), the global `ApprovalGate`, the socket manager, native-bus
/// handlers. Booting in the lib test binary leaked all of them into every
/// test that ran afterwards under `--test-threads=1`: `all_tools` lost its
/// acting tool groups, the tinyflows HTTP adapter hit the origin-label gate
/// instead of the SSRF guard, memory-binding tests saw a workspace they never
/// set up. The parent seeds the store and verifies the sweep; only the child
/// touches the singletons, and it exits with them.
#[tokio::test]
async fn a_build_only_runtime_is_swept_before_it_can_be_invoked() {
    if std::env::var_os(REAPER_BOOT_CHILD_ENV).is_some() {
        boot_child().await;
        return;
    }

    let tmp = std::env::temp_dir().join(format!("oh-reaper-boot-{}", uuid::Uuid::new_v4()));
    // `OPENHUMAN_WORKSPACE` names the root; the resolved workspace is the
    // `workspace` directory under it, which is what the sweep will read.
    let workspace = tmp.join("workspace");
    crate::storage::local::use_persistent_default_for_test(&workspace);
    let store = crate::storage::sessions::status_store_for(&workspace).expect("status store");
    let orphan = seed_status(&store, ExecutionStatus::Running).await;
    assert_eq!(store.list_active().await.unwrap().len(), 1);

    let exe = std::env::current_exe().expect("test binary path");
    let output = std::process::Command::new(exe)
        .args([
            "--exact",
            "agent::tinyagents::reaper::tests::\
             a_build_only_runtime_is_swept_before_it_can_be_invoked",
            "--test-threads=1",
            "--nocapture",
        ])
        .env(REAPER_BOOT_CHILD_ENV, "1")
        .env("OPENHUMAN_WORKSPACE", &tmp)
        .output()
        .expect("spawn the boot child");
    assert!(
        output.status.success(),
        "the boot child must build a core and pass its own assertions\n\
         --- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // The child writes the durable store in a separate process. Reopen it for
    // verification instead of reading through the parent's handle, whose
    // refactored KV layer may still serve the status it cached while seeding.
    let after_store = crate::storage::sessions::status_store_for(&workspace).expect("status store");
    let after = after_store
        .get_status(&orphan)
        .await
        .unwrap()
        .expect("the seeded run is still readable");
    assert_eq!(
        after.status,
        ExecutionStatus::Cancelled,
        "the sweep build() started must reap the orphan"
    );
    assert_eq!(after.error.as_deref(), Some(ORPHAN_REAP_REASON));
    assert!(after_store.list_active().await.unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Set on the re-invoked test binary so the test body knows it is the boot
/// child rather than the seeding/verifying parent.
const REAPER_BOOT_CHILD_ENV: &str = "OPENHUMAN_TEST_REAPER_BOOT_CHILD";

/// The child half of `a_build_only_runtime_is_swept_before_it_can_be_invoked`:
/// build a runtime with no transport and no services — the shape
/// `crates/openhuman-embed/examples/lean_headless.rs` documents — against the `OPENHUMAN_WORKSPACE`
/// the parent exported, and check the build resolved that workspace. The
/// parent reads the sweep's effect out of the store afterwards.
async fn boot_child() {
    use crate::core::runtime::{CoreBuilder, DomainSet, ServiceSet};
    use crate::core::types::HostKind;

    let root = std::path::PathBuf::from(
        std::env::var_os("OPENHUMAN_WORKSPACE").expect("parent exports OPENHUMAN_WORKSPACE"),
    );
    let workspace = root.join("workspace");
    crate::storage::local::use_persistent_default_for_test(&workspace);
    hold_startup_sweeps_for_test();
    let built = CoreBuilder::new(HostKind::Cli)
        .services(ServiceSet::none())
        .domains(DomainSet::harness())
        .build()
        .await
        .expect("a headless build succeeds");
    assert_eq!(
        built.context().workspace_dir().ok(),
        Some(root.join("workspace")),
        "the build must have resolved the workspace the parent seeded"
    );
    // The sweep is held, so the orphan is still active: build() returned
    // without waiting for it.
    let store = crate::storage::sessions::status_store_for(&workspace).expect("status store");
    assert_eq!(
        store.list_active().await.unwrap().len(),
        1,
        "build() must return before the sweep runs"
    );
    release_startup_sweeps_for_test();
    assert_eq!(built.wait_for_startup_sweep().await, Some(1));
    assert_eq!(built.wait_for_startup_sweep().await, None);
}

/// A context derived for `agent` and registered as that agent's live one, the
/// way an embed agent is. Each test uses its own agent ids.
fn live_agent(agent: &str) -> std::sync::Arc<crate::core::runtime::CoreContext> {
    use crate::core::runtime::{AgentContextRegistry, ContextOverlay, CoreContext, DomainSet};
    let context = CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::full(),
            Default::default(),
        )
        .session_agent(agent),
    );
    AgentContextRegistry::register(agent, &context);
    context
}

/// Opening a scope's status store prepares its collections, so a sweep that
/// only decides per scope to do nothing still costs one preparation per
/// agent. Neither the `local` scope nor any recorded agent is visited when the
/// backend is shared.
#[tokio::test]
async fn a_shared_backend_sweep_visits_no_scope() {
    let _guard = crate::storage::test_support::install_backend_for_test(
        crate::storage::test_support::memory_backend(),
    );
    crate::storage::install_with_driver("mongodb", crate::storage::test_support::memory_backend());
    assert!(crate::storage::installed_is_shared());
    let agents = ["reaper-shared-a", "reaper-shared-b", "reaper-shared-c"];
    let contexts: Vec<_> = agents.iter().map(|agent| live_agent(agent)).collect();
    let tmp = std::env::temp_dir().join(format!("oh-reaper-{}", uuid::Uuid::new_v4()));

    let outcome = sweep(&tmp, None).await;

    assert_eq!(
        outcome,
        SweepOutcome {
            scopes: 0,
            reaped: 0
        }
    );
    for (agent, context) in agents.iter().zip(&contexts) {
        crate::core::runtime::AgentContextRegistry::deregister(agent, context);
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The counterpart: a backend one process owns is swept scope by scope, so the
/// shared-backend test above is not passing for want of agents to visit.
#[tokio::test]
async fn a_local_backend_sweep_visits_every_agent_scope() {
    let _guard = crate::storage::test_support::install_backend_for_test(
        crate::storage::test_support::memory_backend(),
    );
    let agents = ["reaper-local-a", "reaper-local-b", "reaper-local-c"];
    let contexts: Vec<_> = agents.iter().map(|agent| live_agent(agent)).collect();
    let tmp = std::env::temp_dir().join(format!("oh-reaper-{}", uuid::Uuid::new_v4()));

    let outcome = sweep(&tmp, None).await;

    assert!(outcome.scopes >= agents.len(), "{outcome:?}");
    for (agent, context) in agents.iter().zip(&contexts) {
        crate::core::runtime::AgentContextRegistry::deregister(agent, context);
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
