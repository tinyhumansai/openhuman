use super::invoker::{AgentInvoker, InvocationOutput};
use super::ops::{run_job, submit_run};
use super::store::JobStore;
use super::types::{JobStatus, RunPayload};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

struct StaticInvoker {
    response: Result<InvocationOutput, String>,
}

#[async_trait]
impl AgentInvoker for StaticInvoker {
    async fn invoke(
        &self,
        _thread_id: Option<&str>,
        _message: &str,
    ) -> Result<InvocationOutput, String> {
        self.response.clone()
    }
}

struct BlockingInvoker {
    gate: Arc<Notify>,
}

#[async_trait]
impl AgentInvoker for BlockingInvoker {
    async fn invoke(
        &self,
        _thread_id: Option<&str>,
        _message: &str,
    ) -> Result<InvocationOutput, String> {
        // Block until the test releases us — used to assert running status.
        self.gate.notified().await;
        Ok(InvocationOutput {
            assistant_message: "released".into(),
            thread_id: "t".into(),
        })
    }
}

#[tokio::test]
async fn submit_run_returns_pending_job_immediately() {
    let store = JobStore::new(Duration::from_secs(3600));
    let invoker = Arc::new(BlockingInvoker {
        gate: Arc::new(Notify::new()),
    });
    let id = submit_run(
        store.clone(),
        invoker,
        RunPayload {
            message: "hi".into(),
            thread_id: None,
        },
        Duration::from_secs(60),
    );
    let view = store.get(&id).expect("inserted");
    // Status is Pending or Running depending on scheduling — both are fine.
    assert!(matches!(
        view.status,
        JobStatus::Pending | JobStatus::Running
    ));
}

#[tokio::test]
async fn run_job_happy_path_marks_completed_with_message() {
    let store = JobStore::new(Duration::from_secs(3600));
    let invoker = Arc::new(StaticInvoker {
        response: Ok(InvocationOutput {
            assistant_message: "hello, world".into(),
            thread_id: "t-42".into(),
        }),
    });
    let id = store.insert_pending();
    run_job(
        store.clone(),
        invoker,
        id.clone(),
        RunPayload {
            message: "ping".into(),
            thread_id: None,
        },
        Duration::from_secs(5),
    )
    .await;
    let view = store.get(&id).unwrap();
    assert_eq!(view.status, JobStatus::Completed);
    let res = view.result.unwrap();
    assert_eq!(res.message, "hello, world");
    assert_eq!(res.thread_id, "t-42");
}

#[tokio::test]
async fn run_job_invoker_error_marks_failed() {
    let store = JobStore::new(Duration::from_secs(3600));
    let invoker = Arc::new(StaticInvoker {
        response: Err("upstream down".into()),
    });
    let id = store.insert_pending();
    run_job(
        store.clone(),
        invoker,
        id.clone(),
        RunPayload {
            message: "ping".into(),
            thread_id: None,
        },
        Duration::from_secs(5),
    )
    .await;
    let view = store.get(&id).unwrap();
    assert_eq!(view.status, JobStatus::Failed);
    assert_eq!(view.error.as_deref(), Some("upstream down"));
    assert!(view.result.is_none());
}

#[tokio::test]
async fn run_job_timeout_marks_failed_with_timeout_message() {
    let store = JobStore::new(Duration::from_secs(3600));
    let gate = Arc::new(Notify::new());
    let invoker = Arc::new(BlockingInvoker { gate });
    let id = store.insert_pending();
    run_job(
        store.clone(),
        invoker,
        id.clone(),
        RunPayload {
            message: "ping".into(),
            thread_id: None,
        },
        Duration::from_millis(20),
    )
    .await;
    let view = store.get(&id).unwrap();
    assert_eq!(view.status, JobStatus::Failed);
    let err = view.error.unwrap();
    assert!(
        err.contains("timeout"),
        "expected timeout error, got: {err}"
    );
}
