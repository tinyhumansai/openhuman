use super::*;
// The callback adapter, driven the way the module drives it: over the bus, on
// the contract's `ScheduleHost` interface, against the real (linked) module.

use crate::cron::handlers::{
    kind_handler, register_kind, JobKindHandler, JobOutcome, JobRunContext, KIND_REGISTRY_TEST_LOCK,
};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;
use tinyflows_bus::schedule::types::{DeliveryConfig, DeliveryStatus, Schedule};
use tinyflows_bus::schedule::{callbacks, DeliveryProjection, CALLBACK_INTERFACE};

pub(super) fn config(tmp: &TempDir) -> Config {
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
    let result = crate::cron::add_shell_job(
        config,
        None,
        Schedule::Every {
            every_ms: 3_600_000,
        },
        command,
    );
    if let Err(error) = &result {
        eprintln!("add_shell_job failed: {error:#}");
    }
    result.unwrap()
}

pub(super) fn flow_job(config: &Config, flow_id: &str) -> CronJob {
    crate::cron::add_flow_schedule_job(
        config,
        flow_id,
        Schedule::Every {
            every_ms: 3_600_000,
        },
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
    let _kinds = KIND_REGISTRY_TEST_LOCK.lock().await;
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
    let _kinds = KIND_REGISTRY_TEST_LOCK.lock().await;
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
    let _kinds = KIND_REGISTRY_TEST_LOCK.lock().await;
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
    let flow = flow_job(&config, "some-flow");
    let trigger: Attempt = proxy
        .call(callbacks::SCHEDULE_TRIGGER, (&flow, "run-d"))
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
        raw_detail: None,
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
        raw_detail: None,
    };
    let status: String = proxy
        .call(callbacks::SEND, (&job, "run-f", &attempt))
        .await
        .unwrap();
    assert_eq!(
        status, "not_requested",
        "delivery mode `none` sends nothing"
    );
    assert!(DeliveryStatus::parse(&status).is_some());
    let projection = DeliveryProjection {
        job,
        run_id: "run-f".into(),
        attempt,
        success: true,
        delivery_status: Some(DeliveryStatus::NotRequested),
    };
    proxy
        .call::<()>(callbacks::DELIVER, (&projection,))
        .await
        .unwrap();
}

#[tokio::test]
async fn scheduled_announce_returns_failed_when_its_channel_is_unavailable() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let (_host, proxy) = host(&config, false).await;
    let mut job = shell_job(&config, "echo nothing");
    job.delivery = DeliveryConfig {
        mode: "announce".into(),
        channel: Some(format!("missing-{}", uuid::Uuid::new_v4())),
        to: Some("target".into()),
        best_effort: false,
    };
    let attempt = Attempt {
        success: true,
        output: "result".into(),
        retryable: true,
        raw_detail: None,
    };

    let status: String = proxy
        .call(callbacks::SEND, (&job, "run-announce", &attempt))
        .await
        .unwrap();

    assert_eq!(
        status, "failed",
        "the scheduler must receive actual send status"
    );
    assert!(DeliveryStatus::parse(&status).is_some());
    let projection = DeliveryProjection {
        job,
        run_id: "run-announce".into(),
        attempt,
        success: false,
        delivery_status: Some(DeliveryStatus::Failed),
    };
    proxy
        .call::<()>(callbacks::DELIVER, (&projection,))
        .await
        .unwrap();
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
        .call(callbacks::SCHEDULE_TRIGGER, (&row, "run-g"))
        .await
        .unwrap();
    assert!(attempt.success, "{}", attempt.output);
    assert!(attempt.output.contains("completed"), "{}", attempt.output);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn schedule_trigger_checks_agent_only_limits_before_running_a_flow_handler() {
    let tmp = TempDir::new().unwrap();
    let mut config = config(&tmp);
    let job = flow_job(&config, "issue56-flow:");
    config.cron.agent_jobs_only = true;

    let (_host, proxy) = host(&config, false).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let _handler = register_kind(
        JobType::Flow,
        Arc::new(Marker {
            prefix: "issue56-flow:",
            inner: kind_handler(&JobType::Flow).unwrap(),
            calls: Arc::clone(&calls),
            permanent: false,
        }),
    );
    let attempt: Attempt = proxy
        .call(callbacks::SCHEDULE_TRIGGER, (&job, "run-issue56"))
        .await
        .unwrap();

    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the flow handler never ran"
    );
    assert!(!attempt.success);
    assert!(
        attempt.output.contains("only agent cron jobs"),
        "{}",
        attempt.output
    );
}

#[cfg(feature = "flows")]
#[tokio::test]
async fn schedule_trigger_for_an_unknown_flow_is_answered_not_failed() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let (_host, proxy) = host(&config, false).await;
    let job = flow_job(&config, "flow-never-saved");
    crate::cron::remove_job(&config, &job.id).unwrap();
    let attempt: Attempt = proxy
        .call(callbacks::SCHEDULE_TRIGGER, (&job, "run-h"))
        .await
        .unwrap();
    assert!(attempt.success, "{}", attempt.output);
    assert!(
        attempt.output.contains("flow schedule tick emitted"),
        "{}",
        attempt.output
    );
}

#[cfg(feature = "flows")]
#[tokio::test]
async fn refreshing_binding_config_updates_existing_flow_subscriber() {
    use tokio::sync::{mpsc, oneshot};

    let tmp = TempDir::new().unwrap();
    let initial = config(&tmp);
    let refreshed = Config {
        action_dir: tmp.path().join("refreshed-action-root"),
        ..initial.clone()
    };
    let binding = Binding::new(
        &DispatchCtx::ambient(),
        &initial,
        Arc::new(SecurityPolicy::from_config(
            &initial.autonomy,
            &initial.workspace_dir,
            &initial.action_dir,
        )),
    );
    let (seen_tx, mut seen_rx) = mpsc::unbounded_channel();
    let (started_tx, started_rx) = oneshot::channel();
    let started_tx = Arc::new(std::sync::Mutex::new(Some(started_tx)));
    let (release_tx, release_rx) = oneshot::channel();
    let release_rx = Arc::new(tokio::sync::Mutex::new(Some(release_rx)));
    let subscriber = crate::flows::bus::FlowTriggerSubscriber::with_runner_and_config_source(
        Arc::clone(&binding.config),
        1,
        move |_ctx, config, _request| {
            let seen_tx = seen_tx.clone();
            let started_tx = Arc::clone(&started_tx);
            let release_rx = Arc::clone(&release_rx);
            async move {
                if let Some(started_tx) = started_tx.lock().unwrap().take() {
                    let _ = started_tx.send(());
                }
                if let Some(release) = release_rx.lock().await.take() {
                    let _ = release.await;
                }
                seen_tx.send(config.action_dir.clone()).unwrap();
            }
        },
    );
    binding
        .flows
        .set(subscriber)
        .ok()
        .expect("subscriber initialized once");

    let ctx = DispatchCtx::ambient();
    binding
        .flows()
        .dispatch_test_app_event(&ctx, "refresh-flow", serde_json::json!({"event": "first"}))
        .await;
    started_rx.await.unwrap();
    binding.set_config(&refreshed);
    binding
        .flows()
        .dispatch_test_app_event(&ctx, "refresh-flow", serde_json::json!({"event": "second"}))
        .await;
    release_tx.send(()).unwrap();
    assert_eq!(seen_rx.recv().await, Some(initial.action_dir.clone()));
    assert_eq!(seen_rx.recv().await, Some(refreshed.action_dir));
}

#[tokio::test]
async fn triggered_run_guard_keeps_a_long_lived_run_deduplicated_past_1024_starts() {
    use crate::core::events::DomainEvent;
    use tinybus::EventHandler;
    use tokio::sync::mpsc;

    enum Seen {
        Trigger(String),
        Barrier,
    }
    struct Events(mpsc::UnboundedSender<Seen>);
    #[async_trait]
    impl EventHandler<DomainEvent> for Events {
        fn name(&self) -> &str {
            "test::cron::trigger_guard_overflow"
        }
        async fn handle(&self, event: &DomainEvent) {
            if let DomainEvent::CronJobTriggered { job_id, .. } = event {
                if job_id == "issue57-overflow-barrier" {
                    let _ = self.0.send(Seen::Barrier);
                } else {
                    let _ = self.0.send(Seen::Trigger(job_id.clone()));
                }
            }
        }
    }

    crate::core::bus::init().await.unwrap();
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let _subscription = crate::core::bus::BUS
        .subscribe(Arc::new(Events(events_tx)))
        .unwrap();
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let binding = Binding::new(
        &DispatchCtx::ambient(),
        &config,
        Arc::new(SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
            &config.action_dir,
        )),
    );
    let now = chrono::Utc::now();
    let job = CronJob {
        claim_generation: 0,
        policy: Default::default(),
        id: "overflow-guard-job".into(),
        expression: "* * * * *".into(),
        schedule: Schedule::Every { every_ms: 60_000 },
        command: "echo guard".into(),
        prompt: None,
        name: None,
        job_type: JobType::Shell,
        session_target: Default::default(),
        model: None,
        agent_id: None,
        enabled: true,
        delivery: Default::default(),
        delete_after_run: false,
        created_at: now,
        next_run: now,
        last_run: None,
        last_status: None,
        last_output: None,
        origin: None,
    };
    binding.publish_triggered_once(&job, "long-lived-run-0");
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(2), events_rx.recv())
            .await
            .expect("first trigger is delivered")
            .expect("subscriber remains open"),
        Seen::Trigger(job_id) if job_id == job.id
    ));
    for index in 1..=1024 {
        let mut other_job = job.clone();
        other_job.id = format!("overflow-job-{index}");
        binding.publish_triggered_once(&other_job, &format!("long-lived-run-{index}"));
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(2), events_rx.recv())
                .await
                .expect("overflow trigger is delivered")
                .expect("subscriber remains open"),
            Seen::Trigger(job_id) if job_id == other_job.id
        ));
    }
    binding.publish_triggered_once(&job, "long-lived-run-0");
    let mut barrier_job = job.clone();
    barrier_job.id = "issue57-overflow-barrier".into();
    binding.publish_triggered_once(&barrier_job, "issue57-overflow-barrier-run");
    let after_retry = tokio::time::timeout(std::time::Duration::from_secs(2), events_rx.recv())
        .await
        .expect("retry and barrier events drain")
        .expect("subscriber remains open");
    assert!(
        matches!(after_retry, Seen::Barrier),
        "a long-lived run retry must not queue another Triggered before the barrier"
    );
}

#[cfg(feature = "flows")]
#[tokio::test]
async fn captured_deleted_flow_job_publishes_trigger_and_completion_identity() {
    use crate::core::events::DomainEvent;
    use tinybus::EventHandler;
    use tokio::sync::mpsc;

    #[derive(Debug, PartialEq, Eq)]
    enum Seen {
        Triggered {
            job_id: String,
            job_name: String,
            job_type: String,
        },
        Completed {
            job_id: String,
            success: bool,
        },
    }

    struct CronEvents(String, mpsc::UnboundedSender<Seen>);
    #[async_trait]
    impl EventHandler<DomainEvent> for CronEvents {
        fn name(&self) -> &str {
            "test::cron::captured_deleted_flow_identity"
        }
        async fn handle(&self, event: &DomainEvent) {
            let seen = match event {
                DomainEvent::CronJobTriggered {
                    job_id,
                    job_name,
                    job_type,
                } => Seen::Triggered {
                    job_id: job_id.clone(),
                    job_name: job_name.clone(),
                    job_type: job_type.clone(),
                },
                DomainEvent::CronJobCompleted {
                    job_id, success, ..
                } => Seen::Completed {
                    job_id: job_id.clone(),
                    success: *success,
                },
                _ => return,
            };
            if match &seen {
                Seen::Triggered { job_id, .. } | Seen::Completed { job_id, .. } => job_id,
            } == &self.0
            {
                let _ = self.1.send(seen);
            }
        }
    }

    crate::core::bus::init().await.unwrap();
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let (_host, proxy) = host(&config, false).await;
    let job = flow_job(&config, "flow-deleted-event");
    let job = crate::cron::update_job(
        &config,
        &job.id,
        tinyflows_bus::schedule::types::CronJobPatch {
            name: Some("captured flow name".into()),
            ..Default::default()
        },
    )
    .unwrap();
    // The module may delete or replace the stored definition after claiming
    // it. ScheduleTrigger and Deliver must keep projecting this captured row.
    crate::cron::remove_job(&config, &job.id).unwrap();
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let _subscription = crate::core::bus::BUS
        .subscribe(Arc::new(CronEvents(job.id.clone(), events_tx)))
        .unwrap();
    let run_id = format!("flow-event-{}", uuid::Uuid::new_v4());
    let attempt: Attempt = proxy
        .call(callbacks::SCHEDULE_TRIGGER, (&job, &run_id))
        .await
        .unwrap();
    let _: Attempt = proxy
        .call(callbacks::SCHEDULE_TRIGGER, (&job, &run_id))
        .await
        .unwrap();
    let delivery_status: String = proxy
        .call(callbacks::SEND, (&job, &run_id, &attempt))
        .await
        .unwrap();
    let success = attempt.success;
    let projection = DeliveryProjection {
        job: job.clone(),
        run_id: run_id.clone(),
        success,
        delivery_status: DeliveryStatus::parse(&delivery_status),
        attempt,
    };
    proxy
        .call::<()>(callbacks::DELIVER, (&projection,))
        .await
        .unwrap();
    let events = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        vec![
            events_rx.recv().await.expect("trigger event remains open"),
            events_rx
                .recv()
                .await
                .expect("completion event remains open"),
        ]
    })
    .await
    .expect("captured run events arrive");
    assert_eq!(
        events,
        vec![
            Seen::Triggered {
                job_id: job.id.clone(),
                job_name: "captured flow name".into(),
                job_type: "Flow".into(),
            },
            Seen::Completed {
                job_id: job.id,
                success,
            },
        ]
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
        raw_detail: None,
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
