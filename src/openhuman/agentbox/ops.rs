//! AgentBox `/run` worker — spawns a tokio task that drives the agent
//! runtime via [`AgentInvoker`] and records the outcome on the [`JobStore`].

use std::time::Duration;
use tokio::time::timeout;

use super::invoker::SharedInvoker;
use super::store::JobStore;
use super::types::{RunPayload, RunResult};

/// Spawn a worker for `payload` and return the new job id immediately.
///
/// Caller-visible behavior: status is `pending` for the brief window before
/// the worker task is scheduled, then transitions to `running` and finally
/// `completed` / `failed`.
pub fn submit_run(
    store: JobStore,
    invoker: SharedInvoker,
    payload: RunPayload,
    job_timeout: Duration,
) -> String {
    let id = store.insert_pending();
    let id_clone = id.clone();
    tokio::spawn(async move {
        run_job(store, invoker, id_clone, payload, job_timeout).await;
    });
    id
}

/// Run a single job synchronously inside the calling task.
///
/// Public so tests can drive it without `tokio::spawn` indirection.
pub async fn run_job(
    store: JobStore,
    invoker: SharedInvoker,
    job_id: String,
    payload: RunPayload,
    job_timeout: Duration,
) {
    store.mark_running(&job_id);
    let message = payload.message;
    let thread_id = payload.thread_id;

    let invocation = invoker.invoke(thread_id.as_deref(), &message);
    let outcome = timeout(job_timeout, invocation).await;

    match outcome {
        Ok(Ok(output)) => {
            log::info!(
                "[agentbox] job {} completed thread_id={} reply_len={}",
                job_id,
                output.thread_id,
                output.assistant_message.len()
            );
            store.mark_completed(
                &job_id,
                RunResult {
                    message: output.assistant_message,
                    thread_id: output.thread_id,
                },
            );
        }
        Ok(Err(err)) => {
            log::warn!("[agentbox] job {} failed: {}", job_id, err);
            store.mark_failed(&job_id, err);
        }
        Err(_elapsed) => {
            let secs = job_timeout.as_secs();
            let msg = format!("job timeout after {}s", secs);
            log::warn!("[agentbox] job {} {}", job_id, msg);
            store.mark_failed(&job_id, msg);
        }
    }
}
