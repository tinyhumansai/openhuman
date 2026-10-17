//! Reading a scope's cron store straight through the module, for tests that
//! stage a backend and check what an importer or another writer left in it.

use super::*;
use crate::storage::{Scope, StorageBackend};
use std::sync::Arc;
use tinyflows_bus::schedule::types::CronRun;

fn run<T: Send + DeserializeOwned + 'static>(
    backend: &Arc<dyn StorageBackend>,
    scope: &Scope,
    operation: StoreOperation,
) -> Result<T> {
    let config = Config::default();
    let scope = DocScope::on_backend(&config, Arc::clone(backend), scope.clone());
    crate::storage::block_on_anyhow(async move { store_in(&config, &scope, operation).await })
}

pub(crate) fn list_jobs(backend: &Arc<dyn StorageBackend>, scope: &Scope) -> Result<Vec<CronJob>> {
    run(backend, scope, StoreOperation::List)
}

pub(crate) fn get_job(
    backend: &Arc<dyn StorageBackend>,
    scope: &Scope,
    job_id: &str,
) -> Result<CronJob> {
    run(
        backend,
        scope,
        StoreOperation::Get {
            job_id: job_id.to_string(),
        },
    )
}

pub(crate) fn list_runs(
    backend: &Arc<dyn StorageBackend>,
    scope: &Scope,
    job_id: &str,
) -> Result<Vec<CronRun>> {
    run(
        backend,
        scope,
        StoreOperation::ListRuns {
            job_id: job_id.to_string(),
            limit: 50,
        },
    )
}
