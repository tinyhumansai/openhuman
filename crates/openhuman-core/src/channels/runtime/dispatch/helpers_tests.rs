use super::*;
use crate::core::runtime::DispatchCtx;
use std::sync::Arc;
use tokio::sync::{oneshot, Notify};

struct DeferredTyping {
    started: Notify,
    release: Notify,
    stopped: std::sync::Mutex<Option<oneshot::Sender<String>>>,
}
#[async_trait::async_trait]
impl crate::channels::Channel for DeferredTyping {
    fn name(&self) -> &str {
        "fixture"
    }
    async fn send(&self, _: &crate::channels::SendMessage) -> anyhow::Result<()> {
        Ok(())
    }
    async fn listen(
        &self,
        _: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn begin_typing_refresh(&self, _: &str, _: u64) -> anyhow::Result<Option<String>> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(Some("module-handle".into()))
    }
    async fn end_typing_refresh(&self, handle: &str) -> anyhow::Result<()> {
        if let Some(sender) = self.stopped.lock().unwrap().take() {
            let _ = sender.send(handle.into());
        }
        Ok(())
    }
}

#[tokio::test]
async fn canceled_waiter_closes_handle_returned_by_in_flight_typing_start() {
    let (sender, receiver) = oneshot::channel();
    let channel = Arc::new(DeferredTyping {
        started: Notify::new(),
        release: Notify::new(),
        stopped: std::sync::Mutex::new(Some(sender)),
    });
    let cancellation = tokio_util::sync::CancellationToken::new();
    let waiter = spawn_typing_task(
        &DispatchCtx::unbound(),
        channel.clone(),
        "recipient".into(),
        cancellation.clone(),
    );
    channel.started.notified().await;
    waiter.abort();
    cancellation.cancel();
    channel.release.notify_one();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), receiver)
            .await
            .unwrap()
            .unwrap(),
        "module-handle"
    );
}

struct ProbeTyping(std::sync::Mutex<Option<oneshot::Sender<Option<String>>>>);
#[async_trait::async_trait]
impl crate::channels::Channel for ProbeTyping {
    fn name(&self) -> &str {
        "probe"
    }
    async fn send(&self, _: &crate::channels::SendMessage) -> anyhow::Result<()> {
        Ok(())
    }
    async fn listen(
        &self,
        _: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn begin_typing_refresh(&self, _: &str, _: u64) -> anyhow::Result<Option<String>> {
        let agent = crate::core::runtime::CoreContext::current()
            .and_then(|core| core.session_agent().map(str::to_owned));
        if let Some(sender) = self.0.lock().unwrap().take() {
            let _ = sender.send(agent);
        }
        Ok(None)
    }
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
    let channel = Arc::new(ProbeTyping(std::sync::Mutex::new(Some(sender))));
    let cancellation = tokio_util::sync::CancellationToken::new();
    // The caller's own scope is Ravi's; the typing task was handed Asha's.
    let waiter = CoreContext::scope(ravi, async {
        spawn_typing_task(&asha, channel, "recipient".into(), cancellation.clone())
    })
    .await;
    let seen = tokio::time::timeout(std::time::Duration::from_secs(2), receiver)
        .await
        .unwrap()
        .unwrap();
    cancellation.cancel();
    let _ = waiter.await;
    assert_eq!(seen.as_deref(), Some("u-asha"));
}
