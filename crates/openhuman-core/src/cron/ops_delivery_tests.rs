use super::*;
use crate::cron;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tinyflows_bus::schedule::types::{DeliveryConfig, DeliveryStatus};
use tokio::sync::mpsc;

/// Register a channel bridge with no module objects, so announcing to any
/// channel fails as "channel not available".
pub(super) fn install_failing_telegram_bridge() {
    crate::cron::channel_bridge::register_channel_bridge(
        Arc::new(std::collections::HashMap::new()),
    );
}

#[tokio::test]
async fn run_job_now_keeps_best_effort_failure_successful_and_records_failed_delivery() {
    let tmp = TempDir::new().unwrap();
    let mut config = tests::test_config(&tmp);
    config.channels_config.telegram = Some(crate::config::TelegramConfig {
        bot_token: "test-token".into(),
        chat_id: None,
        allowed_users: vec!["target".into()],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });
    install_failing_telegram_bridge();
    let _handler = crate::cron::system_job_handlers::register(
        "ops-run-now-best-effort-delivery-failure",
        std::sync::Arc::new(|_ctx| Box::pin(async { Ok(()) })),
    );
    let job = tests::single_flight_system_job(&config, "ops-run-now-best-effort-delivery-failure");
    cron::update_job(
        &config,
        &job.id,
        CronJobPatch {
            delivery: Some(DeliveryConfig {
                mode: "announce".into(),
                channel: Some("telegram".into()),
                to: Some("target".into()),
                best_effort: true,
            }),
            ..CronJobPatch::default()
        },
    )
    .unwrap();

    let (success, _output) = run_job_now(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &job.id,
    )
    .await
    .unwrap();

    assert!(
        success,
        "best-effort delivery failure preserves execution success"
    );
    let runs = cron::list_runs(&config, &job.id, 10).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "ok");
    assert_eq!(runs[0].delivery_status, Some(DeliveryStatus::Failed));
}

#[tokio::test]
async fn run_job_now_pairs_trigger_and_completion_after_delivery() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    use crate::core::events::DomainEvent;
    use tinybus::EventHandler;

    struct Events(String, mpsc::UnboundedSender<&'static str>);
    #[async_trait]
    impl EventHandler<DomainEvent> for Events {
        fn name(&self) -> &str {
            "test::cron::run_now_event_pair"
        }
        async fn handle(&self, event: &DomainEvent) {
            let (job_id, label) = match event {
                DomainEvent::CronJobTriggered { job_id, .. } => (job_id, "triggered"),
                DomainEvent::CronJobCompleted { job_id, .. } => (job_id, "completed"),
                _ => return,
            };
            if job_id == &self.0 {
                let _ = self.1.send(label);
            }
        }
    }

    crate::core::bus::init().await.unwrap();
    let tmp = TempDir::new().unwrap();
    let config = tests::test_config(&tmp);
    let name = "ops-run-now-event-pair";
    let _handler =
        crate::cron::system_job_handlers::register(name, Arc::new(|_| Box::pin(async { Ok(()) })));
    let job = tests::single_flight_system_job(&config, name);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let _subscription = crate::core::bus::BUS
        .subscribe(Arc::new(Events(job.id.clone(), tx)))
        .unwrap();

    let (success, _) = run_job_now(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &job.id,
    )
    .await
    .unwrap();
    assert!(success);
    let pair = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut seen = Vec::new();
        loop {
            let event = rx.recv().await.expect("cron event subscriber remains open");
            seen.push(event);
            if event == "completed" {
                break;
            }
        }
        seen
    })
    .await
    .expect("Run Now completion event arrives");
    assert_eq!(pair, vec!["triggered", "completed"]);
    let runs = cron::list_runs(&config, &job.id, 10).unwrap();
    assert_eq!(
        runs[0].delivery_status,
        Some(DeliveryStatus::NotRequested),
        "requested delivery is projected even when the configured mode sends nothing"
    );
}

#[tokio::test]
async fn detached_manual_run_waits_for_durable_admission_after_starting() {
    use tokio::sync::oneshot;

    crate::core::bus::init().await.unwrap();
    let tmp = TempDir::new().unwrap();
    let config = tests::test_config(&tmp);
    let name = format!("detached-admission-{}", uuid::Uuid::new_v4().simple());
    let (started_tx, started_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let release_rx = Arc::new(Mutex::new(Some(release_rx)));
    let _handler = crate::cron::system_job_handlers::register(
        &name,
        Arc::new(move |_| {
            let started = started_tx.lock().unwrap().take();
            let release = release_rx.lock().unwrap().take();
            Box::pin(async move {
                if let Some(started) = started {
                    let _ = started.send(());
                }
                if let Some(release) = release {
                    release.await.map_err(|error| error.to_string())?;
                }
                Ok(())
            })
        }),
    );
    let job = tests::single_flight_system_job(&config, &name);
    let (claim_entered, claim_release) =
        crate::cron::documents_port::pause_manual_claim_for_test(&job.id);
    let run_id = format!("detached-{}", uuid::Uuid::new_v4());
    let dispatch = crate::core::runtime::DispatchCtx::ambient();
    let task_config = config.clone();
    let task_job = job.clone();
    let task_run_id = run_id.clone();
    let call = tokio::spawn(async move {
        crate::cron::scheduler::run_job_now(
            &dispatch,
            &task_config,
            task_job,
            task_run_id,
            false,
            tinyflows_bus::schedule::ManualDelivery::Requested,
        )
        .await
    });

    tokio::time::timeout(std::time::Duration::from_secs(3), claim_entered)
        .await
        .expect("manual claim reaches the controlled storage boundary")
        .expect("claim gate remains open");
    assert!(
        !call.is_finished(),
        "Starting is not admission while the durable claim is blocked"
    );
    let _ = claim_release.send(());
    tokio::time::timeout(std::time::Duration::from_secs(3), started_rx)
        .await
        .expect("the worker proceeds after durable claim")
        .expect("handler start signal remains open");
    let reply = tokio::time::timeout(std::time::Duration::from_secs(3), call)
        .await
        .expect("confirmed running is returned promptly")
        .unwrap()
        .unwrap();
    assert!(matches!(
        reply,
        tinyflows_bus::schedule::RunJobReply::Accepted { run_id: ref id } if id == &run_id
    ));
    let _ = release_tx.send(());
}

#[tokio::test]
async fn manual_run_claims_are_shared_within_a_scope_and_isolated_between_agents() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use tokio::sync::Semaphore;

    crate::core::bus::init().await.unwrap();
    let tmp = TempDir::new().unwrap();
    let config = tests::test_config(&tmp);
    let alpha = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(
            config.clone(),
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .session_agent("cron-alpha"),
    );
    let beta = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(
            config.clone(),
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .session_agent("cron-beta"),
    );
    let name = format!("scoped-claims-{}", uuid::Uuid::new_v4().simple());
    let alpha_job = CoreContext::scope(Arc::clone(&alpha), async {
        tests::single_flight_system_job(&config, &name)
    })
    .await;
    let beta_job = CoreContext::scope(Arc::clone(&beta), async {
        tests::single_flight_system_job(&config, &name)
    })
    .await;
    assert_eq!(alpha_job.id, beta_job.id);

    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let started_tx = Arc::new(started_tx);
    let release = Arc::new(Semaphore::new(0));
    let release_handler = Arc::clone(&release);
    let handler_started = Arc::clone(&started_tx);
    let _handler = crate::cron::system_job_handlers::register(
        &name,
        Arc::new(move |_| {
            let release = Arc::clone(&release_handler);
            let started = Arc::clone(&handler_started);
            Box::pin(async move {
                let _ = started.send(());
                release
                    .acquire()
                    .await
                    .map_err(|error| error.to_string())?
                    .forget();
                Ok(())
            })
        }),
    );
    let alpha_dispatch = DispatchCtx::new(Arc::clone(&alpha));
    let first_ctx = alpha_dispatch.clone();
    let first_config = config.clone();
    let first_job = alpha_job.clone();
    let first_id = format!("same-scope-{}", uuid::Uuid::new_v4());
    let first_run = first_id.clone();
    let first = tokio::spawn(async move {
        crate::cron::scheduler::run_job_now(
            &first_ctx,
            &first_config,
            first_job,
            first_run,
            true,
            tinyflows_bus::schedule::ManualDelivery::NotRequested,
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), started_rx.recv())
        .await
        .expect("first scope's manual worker starts")
        .expect("handler start channel remains open");

    let duplicate = crate::cron::scheduler::run_job_now(
        &alpha_dispatch,
        &config,
        alpha_job.clone(),
        format!("duplicate-{}", uuid::Uuid::new_v4()),
        true,
        tinyflows_bus::schedule::ManualDelivery::NotRequested,
    )
    .await
    .expect("a duplicate claim is a refusal, not a transport error");
    assert!(matches!(
        duplicate,
        tinyflows_bus::schedule::RunJobReply::Refused {
            reason: tinyflows_bus::schedule::RunJobRefusal::AlreadyRunning
        }
    ));

    let beta_dispatch = DispatchCtx::new(Arc::clone(&beta));
    let beta_ctx = beta_dispatch.clone();
    let beta_config = config.clone();
    let beta_job_for_run = beta_job.clone();
    let beta_id = format!("other-scope-{}", uuid::Uuid::new_v4());
    let beta_run_id = beta_id.clone();
    let second = tokio::spawn(async move {
        crate::cron::scheduler::run_job_now(
            &beta_ctx,
            &beta_config,
            beta_job_for_run,
            beta_run_id,
            true,
            tinyflows_bus::schedule::ManualDelivery::NotRequested,
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), started_rx.recv())
        .await
        .expect("another agent scope can start the same job identity")
        .expect("handler start channel remains open");

    release.add_permits(2);
    assert!(matches!(
        first.await.unwrap().unwrap(),
        tinyflows_bus::schedule::RunJobReply::Completed { success: true, .. }
    ));
    assert!(matches!(
        second.await.unwrap().unwrap(),
        tinyflows_bus::schedule::RunJobReply::Completed { success: true, .. }
    ));
    let alpha_runs = CoreContext::scope(Arc::clone(&alpha), async {
        cron::list_runs(&config, &alpha_job.id, 10).unwrap()
    })
    .await;
    let beta_runs = CoreContext::scope(Arc::clone(&beta), async {
        cron::list_runs(&config, &beta_job.id, 10).unwrap()
    })
    .await;
    assert_eq!(alpha_runs[0].run_id.as_deref(), Some(first_id.as_str()));
    assert_eq!(beta_runs[0].run_id.as_deref(), Some(beta_id.as_str()));
}

#[tokio::test]
async fn manual_owner_reconciles_a_lost_committed_ack_before_releasing_context() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};

    crate::core::bus::init().await.unwrap();
    let tmp = TempDir::new().unwrap();
    let config = tests::test_config(&tmp);
    let agent = format!("ack-retention-{}", uuid::Uuid::new_v4().simple());
    let context = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(
            config.clone(),
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .session_agent(agent),
    );
    let weak = Arc::downgrade(&context);
    let name = format!("ack-retention-job-{}", uuid::Uuid::new_v4().simple());
    let job = CoreContext::scope(Arc::clone(&context), async {
        tests::single_flight_system_job(&config, &name)
    })
    .await;
    let _handler =
        crate::cron::system_job_handlers::register(&name, Arc::new(|_| Box::pin(async { Ok(()) })));
    let (ack_committed, allow_lost_reply) =
        crate::cron::scheduler::lose_next_ack_reply_after_commit_for_test(
            config.workspace_dir.clone(),
        );
    let dispatch = DispatchCtx::new(Arc::clone(&context));
    drop(context);

    let result = crate::cron::scheduler::run_job_now(
        &dispatch,
        &config,
        job,
        format!("ack-run-{}", uuid::Uuid::new_v4()),
        true,
        tinyflows_bus::schedule::ManualDelivery::NotRequested,
    )
    .await
    .expect("the completed result is delivered independently from acknowledgement");
    assert!(matches!(
        result,
        tinyflows_bus::schedule::RunJobReply::Completed { success: true, .. }
    ));
    drop(dispatch);
    tokio::time::timeout(std::time::Duration::from_secs(3), ack_committed)
        .await
        .expect("the first acknowledgement committed before its response was lost")
        .expect("ack response loss gate remains open");
    assert!(
        weak.upgrade().is_some(),
        "the callback owner retains its bound context until Ack is reconciled"
    );

    let _ = allow_lost_reply.send(());
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the invocation context is released after Ack reconciliation");
}

#[tokio::test]
async fn manual_runs_use_each_invocations_config_and_security_snapshot() {
    const CHILD: &str = "OPENHUMAN_CRON_MANUAL_CONFIG_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cron::ops::delivery_tests::manual_runs_use_each_invocations_config_and_security_snapshot",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "isolated config/scope child failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            stdout.contains("running 1 test"),
            "child selected no test: {stdout}"
        );
        assert!(
            stdout.contains("test cron::ops::delivery_tests::manual_runs_use_each_invocations_config_and_security_snapshot ... ok"),
            "the expected child test did not pass: {stdout}"
        );
        return;
    }

    use crate::cron::handlers::{register_kind, JobKindHandler, JobOutcome, JobRunContext};
    use tinyflows_bus::schedule::types::{JobType, Schedule};

    crate::core::bus::init().await.unwrap();
    let tmp = TempDir::new().unwrap();
    let mut first_config = tests::test_config(&tmp);
    first_config.autonomy.enabled = true;
    first_config.autonomy.level = crate::security::AutonomyLevel::ReadOnly;
    let mut second_config = first_config.clone();
    second_config.autonomy.enabled = false;
    second_config.action_dir = tmp.path().join("second-action-root");

    struct ConfigSecurityProbe;
    #[async_trait]
    impl JobKindHandler for ConfigSecurityProbe {
        async fn run(&self, cx: &JobRunContext, _job: &CronJob, _run_id: &str) -> JobOutcome {
            JobOutcome::ok(
                serde_json::json!({
                    "can_act": cx.security().can_act(),
                    "action_dir": cx.config().action_dir,
                })
                .to_string(),
            )
        }
    }
    let _handler = register_kind(JobType::Shell, Arc::new(ConfigSecurityProbe));
    let job = cron::add_shell_job(
        &first_config,
        None,
        Schedule::Every {
            every_ms: 3_600_000,
        },
        "unused by the test handler",
    )
    .unwrap();

    let first = crate::cron::scheduler::run_job_now(
        &crate::core::runtime::DispatchCtx::ambient(),
        &first_config,
        job.clone(),
        format!("first-config-{}", uuid::Uuid::new_v4()),
        true,
        tinyflows_bus::schedule::ManualDelivery::NotRequested,
    )
    .await
    .unwrap();
    let second = crate::cron::scheduler::run_job_now(
        &crate::core::runtime::DispatchCtx::ambient(),
        &second_config,
        job,
        format!("second-config-{}", uuid::Uuid::new_v4()),
        true,
        tinyflows_bus::schedule::ManualDelivery::NotRequested,
    )
    .await
    .unwrap();
    let output = |reply| match reply {
        tinyflows_bus::schedule::RunJobReply::Completed {
            success: true,
            attempt,
            ..
        } => serde_json::from_str::<serde_json::Value>(&attempt.output).unwrap(),
        other => panic!("manual run did not complete successfully: {other:?}"),
    };
    let first = output(first);
    let second = output(second);
    assert_eq!(first["can_act"], false);
    assert_eq!(
        first["action_dir"],
        first_config.action_dir.display().to_string()
    );
    assert_eq!(second["can_act"], true);
    assert_eq!(
        second["action_dir"],
        second_config.action_dir.display().to_string()
    );
}

#[tokio::test]
async fn cancelled_waiting_caller_keeps_manual_run_callback_served_until_completion() {
    use crate::core::events::DomainEvent;
    use tinybus::EventHandler;
    use tokio::sync::{mpsc, oneshot};

    struct Completions {
        job_id: String,
        name: String,
        tx: mpsc::UnboundedSender<bool>,
    }
    #[async_trait]
    impl EventHandler<DomainEvent> for Completions {
        fn name(&self) -> &str {
            &self.name
        }
        async fn handle(&self, event: &DomainEvent) {
            if let DomainEvent::CronJobCompleted {
                job_id, success, ..
            } = event
            {
                if job_id == &self.job_id {
                    let _ = self.tx.send(*success);
                }
            }
        }
    }

    crate::core::bus::init().await.unwrap();
    let tmp = TempDir::new().unwrap();
    let config = tests::test_config(&tmp);
    let name = format!("ops-run-now-cancel-{}", uuid::Uuid::new_v4().simple());
    let job = crate::cron::system_jobs::ensure_system_job(
        &config,
        &name,
        tinyflows_bus::schedule::types::Schedule::Every {
            every_ms: 3_600_000,
        },
    )
    .unwrap();
    let (started_tx, started_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let release_rx = Arc::new(Mutex::new(Some(release_rx)));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handler_calls = Arc::clone(&calls);
    let _handler = crate::cron::system_job_handlers::register(
        &name,
        Arc::new(move |_| {
            let first_call = handler_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
            let started = first_call.then(|| started_tx.lock().unwrap().take().unwrap());
            let release = first_call.then(|| release_rx.lock().unwrap().take().unwrap());
            Box::pin(async move {
                if let Some(started) = started {
                    let _ = started.send(());
                }
                if let Some(release) = release {
                    release.await.map_err(|error| error.to_string())?;
                }
                Ok(())
            })
        }),
    );

    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let _subscription = crate::core::bus::BUS
        .subscribe(Arc::new(Completions {
            job_id: job.id.clone(),
            name: format!("test::cron::cancelled_waiter::{}", job.id),
            tx: events_tx,
        }))
        .unwrap();
    let first_run_id = format!("cancelled-{}", uuid::Uuid::new_v4());
    let first_ctx = crate::core::runtime::DispatchCtx::ambient();
    let first_config = config.clone();
    let first_job = job.clone();
    let first_id = first_run_id.clone();
    let waiting_call = tokio::spawn(async move {
        crate::cron::scheduler::run_job_now(
            &first_ctx,
            &first_config,
            first_job,
            first_id,
            true,
            tinyflows_bus::schedule::ManualDelivery::Requested,
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), started_rx)
        .await
        .expect("the module started the admitted system job")
        .expect("the job-start signal remains open");

    waiting_call.abort();
    assert!(waiting_call.await.unwrap_err().is_cancelled());
    let _ = release_tx.send(());

    let completed = tokio::time::timeout(std::time::Duration::from_secs(3), events_rx.recv()).await;
    assert_eq!(
        completed,
        Ok(Some(true)),
        "cancelling the waiter must not orphan the module worker before terminal projection"
    );
    let runs = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let runs = cron::list_runs(&config, &job.id, 10).unwrap();
            if runs.iter().any(|run| run.status == "ok") {
                break runs;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the detached module worker commits its terminal run");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "ok");
    assert_eq!(runs[0].run_id.as_deref(), Some(first_run_id.as_str()));
}
