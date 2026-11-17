use super::*;

#[tokio::test]
async fn schedule_tick_is_coalesced_while_a_trigger_run_is_active() {
    use std::sync::Mutex;
    use tokio::sync::{mpsc, oneshot};

    let tmp = tempfile::TempDir::new().unwrap();
    let (started_tx, started_rx) = oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let (release_tx, release_rx) = oneshot::channel();
    let release_rx = Arc::new(tokio::sync::Mutex::new(Some(release_rx)));
    let (runs_tx, mut runs_rx) = mpsc::unbounded_channel();
    let runner = move |_ctx: crate::core::runtime::DispatchCtx,
                       _config: Arc<Config>,
                       request: crate::flows::bus::trigger::TriggerRunRequest| {
        let started_tx = Arc::clone(&started_tx);
        let release_rx = Arc::clone(&release_rx);
        let runs_tx = runs_tx.clone();
        async move {
            if let Some(started_tx) = started_tx.lock().unwrap().take() {
                let _ = started_tx.send(());
            }
            if let Some(release) = release_rx.lock().await.take() {
                let _ = release.await;
            }
            runs_tx.send(request.trigger).unwrap();
        }
    };
    let sub = FlowTriggerSubscriber::with_runner(test_config(&tmp), 1, runner);
    let ctx = crate::core::runtime::DispatchCtx::host_default();

    sub.dispatch_test_schedule(&ctx, "f1").await;
    started_rx.await.unwrap();
    sub.dispatch_test_schedule(&ctx, "f1").await;
    release_tx.send(()).unwrap();
    assert_eq!(
        runs_rx.recv().await,
        Some(tinyflows_bus::runs::FlowRunTrigger::Schedule)
    );
    assert!(
        runs_rx.try_recv().is_err(),
        "a busy schedule tick must be dropped"
    );
}

#[tokio::test]
async fn app_event_runs_queue_fifo_per_tenant_while_other_tenants_progress() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use std::sync::Mutex;
    use tokio::sync::{mpsc, oneshot};

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let (completed_tx, mut completed_rx) = mpsc::unbounded_channel();
    let (started_tx, started_rx) = oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let (release_first_tx, release_first_rx) = oneshot::channel();
    let release_first_rx = Arc::new(tokio::sync::Mutex::new(Some(release_first_rx)));
    let runner = move |ctx: DispatchCtx,
                       _config: Arc<Config>,
                       request: crate::flows::bus::trigger::TriggerRunRequest| {
        let completed_tx = completed_tx.clone();
        let started_tx = Arc::clone(&started_tx);
        let release_first_rx = Arc::clone(&release_first_rx);
        async move {
            let tenant = ctx.tenant().unwrap();
            let event = request.input["event"].as_str().unwrap().to_owned();
            if event == "a1" {
                if let Some(started_tx) = started_tx.lock().unwrap().take() {
                    let _ = started_tx.send(());
                }
                if let Some(release) = release_first_rx.lock().await.take() {
                    let _ = release.await;
                }
            }
            completed_tx
                .send((
                    format!(
                        "{}/{}",
                        tenant.profile.unwrap_or_default(),
                        tenant.agent.unwrap_or_default()
                    ),
                    request.flow_id,
                    event,
                ))
                .unwrap();
        }
    };
    let subscriber = FlowTriggerSubscriber::with_runner(config, 1, runner);
    let context = |profile: &str, agent: &str| {
        DispatchCtx::new(
            CoreContext::for_test(DomainSet::full(), None).derive_with(
                ContextOverlay::new(Config::default(), DomainSet::full(), Default::default())
                    .profile(profile)
                    .session_agent(agent),
            ),
        )
    };
    let tenant_a = context("profile-a", "agent-a");
    let tenant_b = context("profile-b", "agent-a");
    let tenant_c = context("profile-a", "agent-c");

    subscriber
        .dispatch_test_app_event(&tenant_a, "shared-flow", json!({"event": "a1"}))
        .await;
    started_rx.await.unwrap();
    subscriber
        .dispatch_test_app_event(&tenant_a, "shared-flow", json!({"event": "a2"}))
        .await;
    subscriber
        .dispatch_test_app_event(&tenant_b, "shared-flow", json!({"event": "b1"}))
        .await;
    subscriber
        .dispatch_test_app_event(&tenant_c, "shared-flow", json!({"event": "c1"}))
        .await;

    let mut other_tenant_runs = [
        receive_trigger_run(&mut completed_rx).await,
        receive_trigger_run(&mut completed_rx).await,
    ];
    other_tenant_runs.sort();
    assert_eq!(
        other_tenant_runs,
        [
            (
                "profile-a/agent-c".to_string(),
                "shared-flow".to_string(),
                "c1".to_string()
            ),
            (
                "profile-b/agent-a".to_string(),
                "shared-flow".to_string(),
                "b1".to_string()
            ),
        ],
        "a held run must not block the same flow ID for another profile or agent"
    );
    release_first_tx.send(()).unwrap();
    assert_eq!(
        receive_trigger_run(&mut completed_rx).await,
        (
            "profile-a/agent-a".to_string(),
            "shared-flow".to_string(),
            "a1".to_string()
        )
    );
    assert_eq!(
        receive_trigger_run(&mut completed_rx).await,
        (
            "profile-a/agent-a".to_string(),
            "shared-flow".to_string(),
            "a2".to_string()
        ),
        "app_event payloads for one tenant and flow must execute FIFO"
    );
}

async fn receive_trigger_run(
    receiver: &mut tokio::sync::mpsc::UnboundedReceiver<(String, String, String)>,
) -> (String, String, String) {
    tokio::time::timeout(std::time::Duration::from_secs(2), receiver.recv())
        .await
        .expect("queued trigger did not finish within the hang guard")
        .unwrap()
}

#[tokio::test]
async fn full_app_event_queue_backpressures_until_a_slot_opens() {
    use std::sync::Mutex;
    use tokio::sync::{mpsc, oneshot};

    let tmp = tempfile::TempDir::new().unwrap();
    let (started_tx, started_rx) = oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let (release_tx, release_rx) = oneshot::channel();
    let release_rx = Arc::new(tokio::sync::Mutex::new(Some(release_rx)));
    let (runs_tx, mut runs_rx) = mpsc::unbounded_channel();
    let runner = move |_ctx: crate::core::runtime::DispatchCtx,
                       _config: Arc<Config>,
                       request: crate::flows::bus::trigger::TriggerRunRequest| {
        let started_tx = Arc::clone(&started_tx);
        let release_rx = Arc::clone(&release_rx);
        let runs_tx = runs_tx.clone();
        async move {
            let event = request.input["event"].as_str().unwrap().to_owned();
            if event == "a1" {
                if let Some(started_tx) = started_tx.lock().unwrap().take() {
                    let _ = started_tx.send(());
                }
                if let Some(release) = release_rx.lock().await.take() {
                    let _ = release.await;
                }
            }
            runs_tx.send(event).unwrap();
        }
    };
    let subscriber = FlowTriggerSubscriber::with_runner(test_config(&tmp), 1, runner);
    let ctx = crate::core::runtime::DispatchCtx::host_default();

    subscriber
        .dispatch_test_app_event(&ctx, "f1", json!({"event": "a1"}))
        .await;
    started_rx.await.unwrap();
    subscriber
        .dispatch_test_app_event(&ctx, "f1", json!({"event": "a2"}))
        .await;
    let third = subscriber.dispatch_test_app_event(&ctx, "f1", json!({"event": "a3"}));
    tokio::pin!(third);
    assert!(
        futures::poll!(third.as_mut()).is_pending(),
        "a full bounded queue must wait for capacity instead of dropping the event"
    );

    release_tx.send(()).unwrap();
    third.await;
    for expected in ["a1", "a2", "a3"] {
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), runs_rx.recv())
                .await
                .expect("backpressured trigger did not finish")
                .unwrap(),
            expected
        );
    }
}

#[tokio::test]
async fn a_panicking_run_does_not_discard_later_accepted_app_events() {
    use std::sync::Mutex;
    use tokio::sync::{mpsc, oneshot};

    let tmp = tempfile::TempDir::new().unwrap();
    let (started_tx, started_rx) = oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let (release_tx, release_rx) = oneshot::channel();
    let release_rx = Arc::new(tokio::sync::Mutex::new(Some(release_rx)));
    let (runs_tx, mut runs_rx) = mpsc::unbounded_channel();
    let runner = move |_ctx: crate::core::runtime::DispatchCtx,
                       _config: Arc<Config>,
                       request: crate::flows::bus::trigger::TriggerRunRequest| {
        let started_tx = Arc::clone(&started_tx);
        let release_rx = Arc::clone(&release_rx);
        let runs_tx = runs_tx.clone();
        async move {
            let event = request.input["event"].as_str().unwrap().to_owned();
            if event == "panic" {
                if let Some(started_tx) = started_tx.lock().unwrap().take() {
                    let _ = started_tx.send(());
                }
                if let Some(release) = release_rx.lock().await.take() {
                    let _ = release.await;
                }
                panic!("synthetic run failure");
            }
            runs_tx.send(event).unwrap();
        }
    };
    let subscriber = FlowTriggerSubscriber::with_runner(test_config(&tmp), 1, runner);
    let ctx = crate::core::runtime::DispatchCtx::host_default();

    subscriber
        .dispatch_test_app_event(&ctx, "f1", json!({"event": "panic"}))
        .await;
    started_rx.await.unwrap();
    subscriber
        .dispatch_test_app_event(&ctx, "f1", json!({"event": "survived"}))
        .await;
    release_tx.send(()).unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), runs_rx.recv())
            .await
            .expect("worker exited after a run panic")
            .unwrap(),
        "survived"
    );
}

#[tokio::test]
async fn arrival_racing_worker_retirement_is_retried_on_a_fresh_queue() {
    use std::sync::Mutex;
    use tokio::sync::{mpsc, oneshot};

    let tmp = tempfile::TempDir::new().unwrap();
    let (started_tx, started_rx) = oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let (release_tx, release_rx) = oneshot::channel();
    let release_rx = Arc::new(tokio::sync::Mutex::new(Some(release_rx)));
    let (runs_tx, mut runs_rx) = mpsc::unbounded_channel();
    let runner = move |_ctx: crate::core::runtime::DispatchCtx,
                       _config: Arc<Config>,
                       request: crate::flows::bus::trigger::TriggerRunRequest| {
        let started_tx = Arc::clone(&started_tx);
        let release_rx = Arc::clone(&release_rx);
        let runs_tx = runs_tx.clone();
        async move {
            let event = request.input["event"].as_str().unwrap().to_owned();
            if event == "first" {
                if let Some(started_tx) = started_tx.lock().unwrap().take() {
                    let _ = started_tx.send(());
                }
                if let Some(release) = release_rx.lock().await.take() {
                    let _ = release.await;
                }
            }
            runs_tx.send(event).unwrap();
        }
    };
    let subscriber = Arc::new(FlowTriggerSubscriber::with_runner(
        test_config(&tmp),
        1,
        runner,
    ));
    let ctx = crate::core::runtime::DispatchCtx::host_default();
    let retiring = subscriber.retirement_notifier_test();
    let retirement_started = retiring.notified();
    tokio::pin!(retirement_started);

    subscriber
        .dispatch_test_app_event(&ctx, "f1", json!({"event": "first"}))
        .await;
    started_rx.await.unwrap();
    let admission = subscriber.hold_queue_admission_test(&ctx, "f1").await;
    release_tx.send(()).unwrap();
    retirement_started.await;

    // The worker notified just before it queued on the held admission gate.
    // Tokio's mutex is FIFO, so its retirement request is ahead of this send.
    let arrival =
        subscriber.dispatch_test_app_event(&ctx, "f1", json!({"event": "after-retirement"}));
    tokio::pin!(arrival);
    assert!(
        futures::poll!(arrival.as_mut()).is_pending(),
        "the arrival must have cloned the retiring queue and be waiting on its admission gate"
    );
    drop(admission);
    tokio::time::timeout(std::time::Duration::from_secs(2), arrival)
        .await
        .expect("the arrival stayed attached to a retired queue");
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), runs_rx.recv())
            .await
            .expect("arrival after worker retirement was lost")
            .unwrap(),
        "first"
    );
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), runs_rx.recv())
            .await
            .expect("retry on the fresh worker was lost")
            .unwrap(),
        "after-retirement"
    );
}

#[test]
fn default_constructs_the_same_as_new() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let a = FlowTriggerSubscriber::new(config.clone());
    let b = FlowTriggerSubscriber::new(config);
    assert_eq!(a.name(), b.name());
}
