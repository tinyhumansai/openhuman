use super::pipeline_is_healthy;

// The full set of statuses `derive_pipeline_status` can return. Kept in sync
// with `memory_tree::tree::rpc::derive_pipeline_status` so a new status forces
// an explicit decision here.
const OPERATIONAL: &[&str] = &["idle", "running", "syncing"];
const NON_OPERATIONAL: &[&str] = &["paused", "error", "degraded"];

#[test]
fn operational_statuses_are_healthy() {
    for status in OPERATIONAL {
        assert!(
            pipeline_is_healthy(status),
            "expected `{status}` to be healthy"
        );
    }
}

#[test]
fn non_operational_statuses_are_not_healthy() {
    for status in NON_OPERATIONAL {
        assert!(
            !pipeline_is_healthy(status),
            "expected `{status}` to be unhealthy"
        );
    }
}

#[test]
fn degraded_is_not_healthy_regression_4691() {
    // #4691: "degraded" previously leaked through the denylist and made the
    // Vault checklist report "Memory pipeline is healthy" while Memory Sync
    // reported "Degraded". It must read as unhealthy.
    assert!(!pipeline_is_healthy("degraded"));
}

#[test]
fn unknown_status_defaults_to_unhealthy() {
    // Allowlist semantics: any future/unexpected status is treated as
    // unhealthy rather than silently reported as healthy.
    assert!(!pipeline_is_healthy("boom"));
    assert!(!pipeline_is_healthy(""));
}
