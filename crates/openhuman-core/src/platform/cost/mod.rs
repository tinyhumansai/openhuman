pub mod budget;
pub mod catalog;
mod global;
pub mod report;
pub mod route;
mod rpc;
mod schemas;
pub mod scope;
mod storage_schema;
pub mod tools;
pub mod tracker;
pub use storage_schema::storage_schema;
mod tracker_documents;
pub mod types;

pub use global::{
    init_global, rebind_global, record_embedding_usage, record_provider_usage,
    record_provider_usage_scoped, seed_tenant_tracker, try_global,
};
pub use route::{route_for_model, CostRoute};
pub use schemas::{
    all_controller_schemas as all_cost_controller_schemas,
    all_registered_controllers as all_cost_registered_controllers,
};
pub use tracker::CostTracker;
pub use types::{
    BudgetStatus, CostDashboard, CostRecord, CostSource, CostSummary, DailyCostEntry, ModelStats,
    TokenUsage, UsageScope,
};

/// Serialises tests that touch the process-global [`CostTracker`].
///
/// The tracker is one process-wide slot shared by every test in this binary,
/// so a test that reads it and a test that installs or rebinds it must not
/// interleave:
/// `rpc_tests::dashboard_query_includes_persisted_record` guards itself with
/// `try_global().is_some()`, and that check-then-use is only sound while no
/// other test can call `init_global` between the two. Every test in this
/// module that installs or reads the global holds this lock.
///
/// Poisoning is ignored on purpose — a panicking test must not cascade into
/// unrelated failures in the tests that follow it.
#[cfg(test)]
pub(super) fn tracker_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}
