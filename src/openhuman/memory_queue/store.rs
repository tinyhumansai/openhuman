//! Product Config adapters over tinycortex's SQLite queue store.

use anyhow::Result;
use rusqlite::Transaction;

use crate::openhuman::config::Config;
use crate::openhuman::memory_tree::health::PipelineFailure;

use super::types::{Job, JobFailure, JobStatus, NewJob};

pub use tinycortex::memory::queue::DEFAULT_LOCK_DURATION_MS;

fn memory_config(config: &Config) -> tinycortex::memory::MemoryConfig {
    crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone())
}

pub fn enqueue(config: &Config, job: &NewJob) -> Result<Option<String>> {
    tinycortex::memory::queue::enqueue(&memory_config(config), job)
}

pub fn enqueue_tx(tx: &Transaction<'_>, job: &NewJob) -> Result<Option<String>> {
    tinycortex::memory::queue::enqueue_tx(tx, job)
}

pub fn claim_next(config: &Config, lock_duration_ms: i64) -> Result<Option<Job>> {
    tinycortex::memory::queue::claim_next(&memory_config(config), lock_duration_ms)
}

pub fn mark_done(config: &Config, job: &Job) -> Result<()> {
    tinycortex::memory::queue::mark_done(&memory_config(config), job)
}

pub fn mark_failed(config: &Config, job: &Job, error: &str) -> Result<()> {
    tinycortex::memory::queue::mark_failed(&memory_config(config), job, error)
}

pub fn mark_failed_typed(
    config: &Config,
    job: &Job,
    error: &str,
    failure: Option<&PipelineFailure>,
) -> Result<()> {
    let failure = failure.map(|failure| JobFailure {
        code: failure.code.as_str(),
        class: failure.class.as_str(),
    });
    tinycortex::memory::queue::mark_failed_typed(
        &memory_config(config),
        job,
        error,
        failure.as_ref(),
    )
}

pub fn mark_deferred(config: &Config, job: &Job, until_ms: i64, reason: &str) -> Result<()> {
    tinycortex::memory::queue::mark_deferred(&memory_config(config), job, until_ms, reason)
}

pub fn recover_stale_locks(config: &Config) -> Result<usize> {
    tinycortex::memory::queue::recover_stale_locks(&memory_config(config))
}

pub fn requeue_failed(config: &Config) -> Result<u64> {
    tinycortex::memory::queue::requeue_failed(&memory_config(config))
}

pub fn requeue_transient_failed(config: &Config) -> Result<u64> {
    tinycortex::memory::queue::requeue_transient_failed(&memory_config(config))
}

pub fn release_running_locks(config: &Config) -> Result<usize> {
    tinycortex::memory::queue::release_running_locks(&memory_config(config))
}

pub fn count_by_status(config: &Config, status: JobStatus) -> Result<u64> {
    tinycortex::memory::queue::count_by_status(&memory_config(config), status)
}

pub fn count_failed_unrecoverable(config: &Config) -> Result<u64> {
    tinycortex::memory::queue::count_failed_unrecoverable(&memory_config(config))
}

pub fn count_total(config: &Config) -> Result<u64> {
    tinycortex::memory::queue::count_total(&memory_config(config))
}

pub fn retry_all_failed(config: &Config) -> Result<u64> {
    tinycortex::memory::queue::retry_all_failed(&memory_config(config))
}

pub fn get_job(config: &Config, id: &str) -> Result<Option<Job>> {
    tinycortex::memory::queue::get_job(&memory_config(config), id)
}
