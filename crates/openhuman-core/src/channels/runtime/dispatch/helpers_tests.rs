use super::*;
use crate::core::runtime::DispatchCtx;
use std::sync::Arc;
use tokio::sync::{oneshot, Notify};

#[tokio::test]
async fn canceled_waiter_closes_handle_returned_by_in_flight_typing_start() {
    let (sender, receiver) = oneshot::channel();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let stopped = Arc::new(std::sync::Mutex::new(Some(sender)));
    let mut test_channel = crate::channels::runtime::TestChannel::new("fixture");
    test_channel.begin_typing_refresh = Some(Arc::new({
        let started = Arc::clone(&started);
        let release = Arc::clone(&release);
        move |_, _| {
            let started = Arc::clone(&started);
            let release = Arc::clone(&release);
            Box::pin(async move {
                started.notify_one();
                release.notified().await;
                Ok(Some("module-handle".into()))
            })
        }
    }));
    test_channel.end_typing_refresh = Some(Arc::new(move |handle| {
        let stopped = Arc::clone(&stopped);
        Box::pin(async move {
            if let Some(sender) = stopped.lock().unwrap().take() {
                let _ = sender.send(handle);
            }
            Ok(())
        })
    }));
    let channel = Arc::new(crate::channels::runtime::ChannelHandle::Test(Arc::new(
        test_channel,
    )));
    let cancellation = tokio_util::sync::CancellationToken::new();
    let waiter = spawn_typing_task(
        &DispatchCtx::unbound(),
        channel.clone(),
        "recipient".into(),
        cancellation.clone(),
    );
    started.notified().await;
    waiter.abort();
    cancellation.cancel();
    release.notify_one();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), receiver)
            .await
            .unwrap()
            .unwrap(),
        "module-handle"
    );
}

#[tokio::test]
async fn typing_task_runs_under_the_handed_context_not_the_active_one() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    let agent_ctx = |agent: &str| {
        CoreContext::for_test(DomainSet::full(), None).derive_with(
            ContextOverlay::new(
                crate::config::Config::default(),
                DomainSet::full(),
                Default::default(),
            )
            .session_agent(agent),
        )
    };
    let asha = DispatchCtx::new(agent_ctx("u-asha"));
    let ravi = agent_ctx("u-ravi");
    let (sender, receiver) = oneshot::channel();
    let sender = Arc::new(std::sync::Mutex::new(Some(sender)));
    let mut test_channel = crate::channels::runtime::TestChannel::new("probe");
    test_channel.begin_typing_refresh = Some(Arc::new(move |_, _| {
        let sender = Arc::clone(&sender);
        Box::pin(async move {
            let agent = crate::core::runtime::CoreContext::current()
                .and_then(|core| core.session_agent().map(str::to_owned));
            if let Some(sender) = sender.lock().unwrap().take() {
                let _ = sender.send(agent);
            }
            Ok(None)
        })
    }));
    let channel = Arc::new(crate::channels::runtime::ChannelHandle::Test(Arc::new(
        test_channel,
    )));
    let cancellation = tokio_util::sync::CancellationToken::new();
    // The caller's own scope is Ravi's; the typing task was handed Asha's.
    let (waiter_tx, waiter_rx) = oneshot::channel();
    CoreContext::scope(ravi, async {
        tokio::task::yield_now().await;
        let waiter = spawn_typing_task(&asha, channel, "recipient".into(), cancellation.clone());
        let _ = waiter_tx.send(waiter);
    })
    .await;
    let waiter = waiter_rx.await.unwrap();
    let seen = tokio::time::timeout(std::time::Duration::from_secs(2), receiver)
        .await
        .unwrap()
        .unwrap();
    cancellation.cancel();
    let _ = waiter.await;
    assert_eq!(seen.as_deref(), Some("u-asha"));
}
