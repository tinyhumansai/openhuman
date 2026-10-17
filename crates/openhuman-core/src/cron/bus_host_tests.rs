//! The callback adapter, driven the way the module drives it: over the bus, on
//! the contract's `ScheduleHost` interface, against the real (linked) module.

use super::*;
use crate::cron::handlers::{
    kind_handler, register_kind, JobKindHandler, JobOutcome, JobRunContext,
};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;
use tinyflows_bus::schedule::types::Schedule;
use tinyflows_bus::schedule::{callbacks, CALLBACK_INTERFACE};

fn config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

async fn host(config: &Config, agent_only: bool) -> (HostRegistration, tinybus::Proxy) {
    module::proxy(config).await.expect("module attaches");
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    let mut binding = Binding::new(&DispatchCtx::ambient(), config, security);
    if agent_only {
        binding = binding.agent_only();
    }
    let registration = register(binding).await.expect("host object is served");
    let address = registration.address();
    let proxy = crate::modules::runtime::runtime()
        .await
        .unwrap()
        .connection()
        .proxy(
            &address.destination,
            &address.object_path,
            CALLBACK_INTERFACE,
        )
        .expect("the contract's callback interface is the one served");
    (registration, proxy)
}

fn shell_job(config: &Config, command: &str) -> CronJob {
    crate::cron::add_shell_job(
        config,
        None,
        Schedule::Every {
            every_ms: 3_600_000,
        },
        command,
    )
    .unwrap()
}

/// Runs jobs whose command starts with its own `prefix` itself and hands every
/// other job to the handler it replaced, so tests running in parallel keep
/// their real shell/flow behaviour.
struct Marker {
    prefix: &'static str,
    inner: Arc<dyn JobKindHandler>,
    calls: Arc<AtomicUsize>,
    permanent: bool,
}

#[async_trait]
impl JobKindHandler for Marker {
    async fn run(&self, cx: &JobRunContext, job: &CronJob, run_id: &str) -> JobOutcome {
        if !job.command.starts_with(self.prefix) {
            return self.inner.run(cx, job, run_id).await;
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = JobOutcome::failed(format!("marker ran {run_id}"));
        if self.permanent {
            outcome.permanent()
        } else {
            outcome
        }
    }
}

#[tokio::test]
async fn execute_runs_the_registered_handler_and_reports_a_retryable_failure() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let (_host, proxy) = host(&config, false).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let _registration = register_kind(
        JobType::Shell,
        Arc::new(Marker {
            prefix: "bus-host-test:retryable:",
            inner: kind_handler(&JobType::Shell).unwrap(),
            calls: Arc::clone(&calls),
            permanent: false,
        }),
    );
    let job = shell_job(&config, "bus-host-test:retryable:");
    let attempt: Attempt = proxy
        .call(callbacks::EXECUTE, (&job, "run-a"))
        .await
        .unwrap();
    assert!(!attempt.success);
    assert!(attempt.retryable, "an ordinary failure may be retried");
    assert_eq!(attempt.output, "marker ran run-a");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_permanent_handler_outcome_stops_the_retry_loop() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let (_host, proxy) = host(&config, false).await;
    let _registration = register_kind(
        JobType::Shell,
        Arc::new(Marker {
            prefix: "bus-host-test:permanent:",
            inner: kind_handler(&JobType::Shell).unwrap(),
            calls: Arc::new(AtomicUsize::new(0)),
            permanent: true,
        }),
    );
    let job = shell_job(&config, "bus-host-test:permanent:");
    let attempt: Attempt = proxy
        .call(callbacks::EXECUTE, (&job, "run-b"))
        .await
        .unwrap();
    assert!(!attempt.success);
    assert!(!attempt.retryable);
}

#[tokio::test]
async fn an_agent_only_scheduler_refuses_shell_jobs_before_running_anything() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let (_host, proxy) = host(&config, true).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let _registration = register_kind(
        JobType::Shell,
        Arc::new(Marker {
            prefix: "bus-host-test:refused:",
            inner: kind_handler(&JobType::Shell).unwrap(),
            calls: Arc::clone(&calls),
            permanent: false,
        }),
    );
    let job = shell_job(&config, "bus-host-test:refused:");
    let attempt: Attempt = proxy
        .call(callbacks::EXECUTE, (&job, "run-c"))
        .await
        .unwrap();
    assert!(!attempt.success);
    assert!(!attempt.retryable);
    assert!(
        attempt.output.contains("only agent cron jobs"),
        "{}",
        attempt.output
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the handler never ran");
    let trigger: Attempt = proxy
        .call(callbacks::SCHEDULE_TRIGGER, ("some-flow", "run-d"))
        .await
        .unwrap();
    assert!(!trigger.success, "flow ticks are refused too");
}

#[tokio::test]
async fn failed_is_accepted_for_any_job() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let (_host, proxy) = host(&config, false).await;
    let job = shell_job(&config, "echo nothing");
    let attempt = Attempt {
        success: false,
        output: "boom".into(),
        retryable: false,
    };
    proxy
        .call::<()>(callbacks::FAILED, (&job, "run-e", &attempt))
        .await
        .unwrap();
}

#[tokio::test]
async fn deliver_answers_with_the_delivery_status_string() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let (_host, proxy) = host(&config, false).await;
    let job = shell_job(&config, "echo nothing");
    let attempt = Attempt {
        success: true,
        output: "result".into(),
        retryable: true,
    };
    let status: String = proxy
        .call(callbacks::DELIVER, (&job, "run-f", &attempt))
        .await
        .unwrap();
    assert_eq!(
        status, "not_requested",
        "delivery mode `none` sends nothing"
    );
    assert!(DeliveryStatus::parse(&status).is_some());
}

#[tokio::test]
async fn schedule_trigger_runs_a_system_job_and_records_the_handler_result() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let (_host, proxy) = host(&config, false).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let _handler = crate::cron::system_job_handlers::register(
        "bus-host-system",
        Arc::new(move |ctx| {
            seen.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                assert_eq!(ctx.name, "bus-host-system");
                Ok(())
            })
        }),
    );
    let row = crate::cron::system_jobs::ensure_system_job(
        &config,
        "bus-host-system",
        Schedule::Every {
            every_ms: 3_600_000,
        },
    )
    .unwrap();
    let attempt: Attempt = proxy
        .call(callbacks::SCHEDULE_TRIGGER, (&row.command, "run-g"))
        .await
        .unwrap();
    assert!(attempt.success, "{}", attempt.output);
    assert!(attempt.output.contains("completed"), "{}", attempt.output);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "flows")]
#[tokio::test]
async fn schedule_trigger_for_an_unknown_flow_is_answered_not_failed() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let (_host, proxy) = host(&config, false).await;
    let attempt: Attempt = proxy
        .call(callbacks::SCHEDULE_TRIGGER, ("flow-never-saved", "run-h"))
        .await
        .unwrap();
    assert!(attempt.success, "{}", attempt.output);
    assert!(
        attempt.output.contains("flow schedule tick emitted"),
        "{}",
        attempt.output
    );
}

#[tokio::test]
async fn dropping_the_registration_stops_serving_the_object() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let (registration, proxy) = host(&config, false).await;
    let job = shell_job(&config, "echo nothing");
    let attempt = Attempt {
        success: true,
        output: String::new(),
        retryable: true,
    };
    drop(registration);
    for _ in 0..100 {
        let reply: Result<(), _> = proxy
            .call(callbacks::FAILED, (&job, "run-i", &attempt))
            .await;
        if reply.is_err() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the host object kept answering after its registration dropped");
}
