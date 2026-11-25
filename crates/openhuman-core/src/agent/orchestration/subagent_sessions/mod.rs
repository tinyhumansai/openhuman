mod ops;
mod storage_schema;
mod store;
pub(crate) use storage_schema::collection_specs;
#[cfg(feature = "legacy-import")]
pub(crate) use storage_schema::upgrade_records;
mod store_documents;
mod tiny_port;
mod tiny_port_epoch;
mod types;

#[cfg(test)]
pub(crate) use ops::cancel_driver_job_by_task;
pub use ops::{
    action_root_key, cancel_driver_job_for_observed_session, cancel_driver_jobs_for_thread,
    close_if_current, find_reusable, list_for_parent, mark_failed, mark_finished,
    normalize_task_key, resolve_driver_lifecycle_key, resolve_lifecycle_key, reuse_decision,
    upsert_running, DriverTaskCancellation,
};
#[cfg(test)]
pub(crate) use tiny_port::OpenHumanTinySubagentSessionStore;
pub(crate) use tiny_port::{host_projection, import_terminal_task_for_context, open_for_context};
pub(crate) use tiny_port_epoch::{parent_epoch_for_context, set_parent_epoch_for_context};
pub use types::{
    DurableSubagentStatus, SubagentSessionSelector, SubagentSessionStore, SubagentSessionUpsert,
};
// The spawn compatibility adapter retains this host projection while the
// remaining lifecycle writers migrate to TinyAgents' durable store.
pub(crate) use types::DurableSubagentSession;
