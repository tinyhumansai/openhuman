//! Tests for the memory exit path.
//!
//! Every test drives `run_exit` with bindings and hooks of its own, never the
//! process-wide snapshot and registry drain `shutdown_for_exit` performs: in a
//! test binary the snapshot is every other test's cached driver, and the
//! registry holds the shared memory engine's own shutdown hook — shutting those
//! down mid-flight hands the other tests the null fallback and a dead store.
//! The gate is exercised on an instance of its own for the same reason: with
//! the process's gate raised, every other test's bind is refused.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::binding::{cached_bindings, for_subtree};
use super::exit::{run_exit, ExitGate, EXIT_BUDGET};
use crate::openhuman::config::schema::MemorySubsystemConfig;

/// With no binding, the exit path is the hooks alone — each run once. What an
/// exit took from the registry is gone from it (`take_hooks` drains), so a
/// second exit — the app-update restart path asks twice — finds nothing to
/// run again.
#[tokio::test]
async fn exit_runs_each_hook_once() {
    let runs = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&runs);
    let hook = crate::core::shutdown::boxed_hook(move || {
        let counter = Arc::clone(&counter);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });

    run_exit(Vec::new(), vec![hook]).await;
    run_exit(Vec::new(), Vec::new()).await;

    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "a hook runs on the exit that took it, and never again"
    );
}

/// The gate that refuses new bindings goes up only once a real server has
/// started — a bare exit (every unit test in this process) must never leave
/// memory refusing to bind — and a server starting again takes it down.
#[test]
fn the_exit_gate_engages_only_behind_a_server() {
    let gate = ExitGate::new();

    gate.raise_if_serving();
    assert!(
        !gate.exiting(),
        "no server ever started, so exit must not gate bindings"
    );

    gate.server_starting();
    gate.raise_if_serving();
    assert!(gate.exiting(), "behind a server, exit refuses new bindings");

    gate.server_starting();
    assert!(
        !gate.exiting(),
        "a server starting again takes the gate down"
    );
}

/// A hook that never answers must not hold the quit: exit returns within its
/// budget (plus the hooks' floor) and the process goes on without it.
#[tokio::test]
async fn exit_is_bounded_even_when_a_hook_hangs() {
    let hang = crate::core::shutdown::boxed_hook(|| async {
        tokio::time::sleep(Duration::from_secs(30)).await;
    });

    let started = std::time::Instant::now();
    run_exit(Vec::new(), vec![hang]).await;

    assert!(
        started.elapsed() < EXIT_BUDGET + Duration::from_secs(2),
        "exit took {:?}; the hanging hook held it past the budget",
        started.elapsed()
    );
}

/// The shell restarts the embedded server in place (a permission refresh, an
/// app update). Exit shuts every cached driver down, so the server that starts
/// next must not be handed those drivers back: exit evicts them, and the next
/// bind builds anew.
#[tokio::test]
async fn exit_evicts_the_drivers_it_shut_down() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cfg = MemorySubsystemConfig {
        driver: "null".into(),
        ..Default::default()
    };
    let before = for_subtree(workspace.path(), "memory", &cfg).expect("binds the null driver");
    assert!(
        cached_bindings()
            .iter()
            .any(|cached| Arc::ptr_eq(cached, &before)),
        "the binding is cached before exit"
    );

    run_exit(vec![Arc::clone(&before)], Vec::new()).await;
    assert!(
        !cached_bindings()
            .iter()
            .any(|cached| Arc::ptr_eq(cached, &before)),
        "exit must drop the driver it shut down"
    );

    let after = for_subtree(workspace.path(), "memory", &cfg).expect("binds again after a restart");
    assert!(
        !Arc::ptr_eq(&before, &after),
        "a restarted server must not be handed back a driver exit already stopped"
    );
}
