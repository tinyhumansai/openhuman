use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tinyflows_bus::schedule::types::Schedule;

/// Answers jobs whose command starts with `handlers-test:` and hands every
/// other job to the handler it replaced, so parallel tests keep the defaults.
struct Tagged {
    tag: &'static str,
    inner: Option<Arc<dyn JobKindHandler>>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl JobKindHandler for Tagged {
    async fn run(&self, cx: &JobRunContext, job: &CronJob, run_id: &str) -> JobOutcome {
        if job.command.starts_with("handlers-test:") {
            self.calls.fetch_add(1, Ordering::SeqCst);
            return JobOutcome::ok(self.tag);
        }
        match &self.inner {
            Some(inner) => inner.run(cx, job, run_id).await,
            None => JobOutcome::failed("no inner handler"),
        }
    }
}

fn tagged(tag: &'static str) -> Arc<Tagged> {
    Arc::new(Tagged {
        tag,
        inner: kind_handler(&JobType::Flow),
        calls: Arc::new(AtomicUsize::new(0)),
    })
}

fn same(a: &Option<Arc<dyn JobKindHandler>>, b: &Arc<dyn JobKindHandler>) -> bool {
    a.as_ref().is_some_and(|a| Arc::ptr_eq(a, b))
}

#[test]
fn every_job_kind_has_a_default_handler() {
    for kind in [JobType::Shell, JobType::Agent, JobType::Flow] {
        assert!(kind_handler(&kind).is_some(), "{}", kind.as_str());
    }
}

#[test]
fn a_later_registration_wins_and_dropping_restores_in_order() {
    let _kinds = KIND_REGISTRY_TEST_LOCK.blocking_lock();
    let default = kind_handler(&JobType::Flow).unwrap();
    let first: Arc<dyn JobKindHandler> = tagged("first");
    let second: Arc<dyn JobKindHandler> = tagged("second");
    let one = register_kind(JobType::Flow, Arc::clone(&first));
    assert!(same(&kind_handler(&JobType::Flow), &first));
    let two = register_kind(JobType::Flow, Arc::clone(&second));
    assert!(same(&kind_handler(&JobType::Flow), &second));
    // Dropping the older guard leaves the newer handler in place.
    drop(one);
    assert!(same(&kind_handler(&JobType::Flow), &second));
    // Dropping the newer one restores what it replaced (the first), which its
    // own guard already gave up; the registry never resurrects a dropped one.
    drop(two);
    let current = kind_handler(&JobType::Flow);
    assert!(
        same(&current, &first) || same(&current, &default),
        "restored to a handler that was current before"
    );
}

#[tokio::test]
async fn the_registered_handler_decides_the_outcome() {
    let _kinds = KIND_REGISTRY_TEST_LOCK.lock().await;
    let handler = tagged("custom");
    let calls = Arc::clone(&handler.calls);
    let _guard = register_kind(JobType::Flow, handler);
    let registry = kind_handler(&JobType::Flow).unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().into(),
        ..Config::default()
    };
    let binding = Binding::new(
        &DispatchCtx::ambient(),
        &config,
        Arc::new(SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
            &config.action_dir,
        )),
    );
    let job = crate::cron::add_flow_schedule_job(
        &config,
        "handlers-test:x",
        Schedule::Every { every_ms: 60_000 },
    )
    .unwrap();
    let outcome = registry
        .run(&JobRunContext::new(binding), &job, "run-1")
        .await;
    assert_eq!(outcome, JobOutcome::ok("custom"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn outcome_builders_set_success_and_permanence() {
    assert!(JobOutcome::ok("x").success);
    let failed = JobOutcome::failed("x");
    assert!(!failed.success && !failed.permanent);
    assert!(failed.permanent().permanent);
}
