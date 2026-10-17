//! Scheduled-job host: the adapters between the product and the `tinyflows`
//! module, which owns the scheduler runtime, the job + run store and the
//! schedule maths behind `tinyflows_bus::schedule`. Core keeps the policy and
//! the pluggable halves: the poll cadence, the `ScheduleHost` callback object
//! and the document port the module calls back, the job-kind handler registry
//! (shell, agent, flow, system jobs), and delivery of job output into the agent
//! / channel pipelines. Shell jobs sandbox through `security::SecurityPolicy`;
//! agent jobs build an `Agent` directly; flow jobs fire the flow's schedule
//! trigger directly.
//! See `README.md` for the full module map.

// Host-condition policy (battery / AC / CPU) that throttles background LLM
// work such as memory digests. The cron poll loop itself does not consult it.

pub mod bus;
pub(crate) mod bus_host;
pub mod channel_bridge;
pub(crate) mod client;
pub mod completion_owner;
pub(crate) mod documents_port;
pub mod handlers;
pub mod host_delivery;
pub mod job_builder;
pub mod limits;
pub mod ops;
pub mod origin;
mod schemas;
pub mod seed;
pub(crate) mod store;
pub mod system_job_handlers;
pub mod system_jobs;
pub mod tools;

pub mod scheduler;

#[cfg(test)]
#[path = "cutover_tests.rs"]
mod cutover_tests;

pub use ops as rpc;
#[cfg(test)]
pub use ops::update_cron_job;
pub use ops::{add_once, add_once_at, pause_job, resume_job};
pub use schemas::{
    all_controller_schemas as all_cron_controller_schemas,
    all_registered_controllers as all_cron_registered_controllers, schemas as cron_schemas,
};
#[allow(unused_imports)]
pub use store::{
    add_agent_job, add_agent_job_from_spec, add_agent_job_with_definition, add_flow_schedule_job,
    add_job, add_shell_job, clear_all_jobs, dedup_named_jobs, delete_queued_runs, due_jobs,
    find_flow_schedule_job, get_job, list_jobs, list_runs, record_last_run, record_run,
    record_run_with_delivery, remove_job, reschedule_after_run, update_job,
};
// The job, run and schedule records are the tinyflows bus contract's; the
// host re-exports them so `cron::Schedule` etc. keep their paths and no type is
// redeclared here. The schedule maths and validation run in the module
// (`client`), so no function is re-exported.
pub use tinyflows_bus::schedule::types::delivery_mode;
pub use tinyflows_bus::schedule::types::{
    ActiveHours, CronJob, CronJobPatch, CronRun, DeliveryConfig, DeliveryStatus, JobOrigin,
    JobType, Schedule, SessionTarget,
};
pub use tinyflows_bus::schedule::{AgentJobSpec, JobPolicy, TooFrequent, MIN_AGENT_JOB_INTERVAL};
