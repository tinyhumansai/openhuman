//! Scheduled-job runtime: cron/human-delay parsing, the job + run store, the
//! polling scheduler, and delivery of job output into the agent / channel
//! pipelines. Shell jobs sandbox through `security::SecurityPolicy`; agent
//! jobs build an `Agent` directly; flow jobs hand off to
//! `flows::bus::FlowTriggerSubscriber` via `DomainEvent::FlowScheduleTick`.
//! See `README.md` for the full module map.

// Host-condition policy (battery / AC / CPU) that throttles background LLM
// work such as memory digests. The cron poll loop itself does not consult it.
pub mod scheduler_gate;

pub mod bus;
pub mod ops;
mod schemas;
pub mod seed;
mod store;
pub mod system_jobs;
pub mod tools;

pub mod scheduler;

pub use ops as rpc;
pub use ops::{add_once, add_once_at, parse_human_delay, pause_job, resume_job, update_cron_job};
// Pure scheduling logic (schedule model, next-run computation) lives in
// `tinyflows-schedule`; the host keeps the scheduler runtime, store, config
// and RPC, and re-exports the upstream names so `cron::Schedule` etc. hold.
pub use schemas::{
    all_controller_schemas as all_cron_controller_schemas,
    all_registered_controllers as all_cron_registered_controllers, schemas as cron_schemas,
};
#[allow(unused_imports)]
pub use store::{
    add_agent_job, add_agent_job_with_definition, add_flow_schedule_job, add_job, add_shell_job,
    clear_all_jobs, dedup_named_jobs, delete_queued_runs, due_jobs, find_flow_schedule_job,
    get_job, list_jobs, list_runs, record_last_run, record_run, remove_job, reschedule_after_run,
    update_job,
};
#[allow(unused_imports)]
pub use tinyflows_schedule::schedule::{
    next_run_for_schedule, normalize_expression, runs_closer_than, schedule_cron_expression,
    validate_agent_schedule, validate_schedule, TooFrequent, MIN_AGENT_JOB_INTERVAL,
};
pub use tinyflows_schedule::types::{
    ActiveHours, CronJob, CronJobPatch, CronRun, DeliveryConfig, JobType, Schedule, SessionTarget,
};
