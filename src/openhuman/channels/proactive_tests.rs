use super::*;
use crate::openhuman::channels::traits::ChannelMessage;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tokio::sync::mpsc;

struct MockChannel {
    name: String,
    send_count: Arc<AtomicUsize>,
    last_idempotency_key: Arc<Mutex<Option<String>>>,
    /// Configured proactive delivery target. `Some` ⇒ the channel can
    /// receive recipient-less proactive sends; `None` ⇒ proactive routing
    /// skips it (models Telegram, which has no stored default chat).
    target: Option<String>,
}

impl MockChannel {
    /// A channel that *can* receive proactive sends (target defaults to its
    /// own name, mirroring Discord's configured `channel_id`).
    fn new(name: &str, send_count: Arc<AtomicUsize>) -> Self {
        Self {
            name: name.to_string(),
            send_count,
            last_idempotency_key: Arc::new(Mutex::new(None)),
            target: Some(name.to_string()),
        }
    }

    /// A channel with no resolvable proactive target (e.g. Telegram).
    fn without_target(name: &str, send_count: Arc<AtomicUsize>) -> Self {
        Self {
            name: name.to_string(),
            send_count,
            last_idempotency_key: Arc::new(Mutex::new(None)),
            target: None,
        }
    }

    fn with_recorder(
        name: &str,
        send_count: Arc<AtomicUsize>,
        last_idempotency_key: Arc<Mutex<Option<String>>>,
    ) -> Self {
        Self {
            name: name.to_string(),
            send_count,
            last_idempotency_key,
            target: Some(name.to_string()),
        }
    }
}

#[async_trait]
impl Channel for MockChannel {
    fn name(&self) -> &str {
        &self.name
    }
    fn proactive_target(&self) -> Option<String> {
        self.target.clone()
    }
    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.send_count.fetch_add(1, Ordering::SeqCst);
        *self
            .last_idempotency_key
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = message.idempotency_key.clone();
        Ok(())
    }
    async fn listen(&self, _tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        Ok(())
    }
}

fn proactive_event() -> DomainEvent {
    DomainEvent::ProactiveMessageRequested {
        source: "cron:test".into(),
        message: "Hello!".into(),
        job_name: Some("test".into()),
    }
}

#[tokio::test]
async fn web_only_does_not_panic() {
    let sub = ProactiveMessageSubscriber::web_only();
    // Should publish to web channel and not panic.
    sub.handle(&proactive_event()).await;
}

#[tokio::test]
async fn routes_to_active_external_channel() {
    let send_count = Arc::new(AtomicUsize::new(0));
    let last_idempotency_key = Arc::new(Mutex::new(None));
    let ch: Arc<dyn Channel> = Arc::new(MockChannel::with_recorder(
        "telegram",
        Arc::clone(&send_count),
        Arc::clone(&last_idempotency_key),
    ));
    let map: HashMap<String, Arc<dyn Channel>> = [("telegram".into(), ch)].into();
    let sub = ProactiveMessageSubscriber::new(Arc::new(map), Some("telegram".into()));

    sub.handle(&proactive_event()).await;

    assert_eq!(send_count.load(Ordering::SeqCst), 1);
    assert!(last_idempotency_key
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_deref()
        .unwrap()
        .starts_with("legacy-send:telegram:"));
}

#[tokio::test]
async fn skips_external_when_channel_has_no_proactive_target() {
    // The active channel is the configured default, but it has no resolvable
    // delivery target (e.g. Telegram with no stored chat). Proactive routing
    // must skip it rather than calling `send` with an empty recipient
    // (#3794 review — Codex P2).
    let send_count = Arc::new(AtomicUsize::new(0));
    let ch: Arc<dyn Channel> = Arc::new(MockChannel::without_target(
        "telegram",
        Arc::clone(&send_count),
    ));
    let map: HashMap<String, Arc<dyn Channel>> = [("telegram".into(), ch)].into();
    let sub = ProactiveMessageSubscriber::new(Arc::new(map), Some("telegram".into()));

    sub.handle(&proactive_event()).await;

    assert_eq!(send_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn skips_external_when_active_is_web() {
    let send_count = Arc::new(AtomicUsize::new(0));
    let ch: Arc<dyn Channel> = Arc::new(MockChannel::new("telegram", Arc::clone(&send_count)));
    let map: HashMap<String, Arc<dyn Channel>> = [("telegram".into(), ch)].into();
    let sub = ProactiveMessageSubscriber::new(Arc::new(map), Some("web".into()));

    sub.handle(&proactive_event()).await;

    // Active channel is "web" — external channel should NOT be called.
    assert_eq!(send_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn skips_external_when_active_is_none() {
    let send_count = Arc::new(AtomicUsize::new(0));
    let ch: Arc<dyn Channel> = Arc::new(MockChannel::new("telegram", Arc::clone(&send_count)));
    let map: HashMap<String, Arc<dyn Channel>> = [("telegram".into(), ch)].into();
    let sub = ProactiveMessageSubscriber::new(Arc::new(map), None);

    sub.handle(&proactive_event()).await;

    assert_eq!(send_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn runtime_update_active_channel() {
    let send_count = Arc::new(AtomicUsize::new(0));
    let ch: Arc<dyn Channel> = Arc::new(MockChannel::new("discord", Arc::clone(&send_count)));
    let map: HashMap<String, Arc<dyn Channel>> = [("discord".into(), ch)].into();
    let sub = ProactiveMessageSubscriber::new(Arc::new(map), None);

    // Initially no active channel — external not called.
    sub.handle(&proactive_event()).await;
    assert_eq!(send_count.load(Ordering::SeqCst), 0);

    // Update at runtime.
    sub.set_active_channel(Some("discord".into()));
    sub.handle(&proactive_event()).await;
    assert_eq!(send_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ignores_non_proactive_events() {
    let send_count = Arc::new(AtomicUsize::new(0));
    let ch: Arc<dyn Channel> = Arc::new(MockChannel::new("telegram", Arc::clone(&send_count)));
    let map: HashMap<String, Arc<dyn Channel>> = [("telegram".into(), ch)].into();
    let sub = ProactiveMessageSubscriber::new(Arc::new(map), Some("telegram".into()));

    sub.handle(&DomainEvent::CronJobTriggered {
        job_id: "j".into(),
        job_name: "test-job".into(),
        job_type: "agent".into(),
    })
    .await;

    assert_eq!(send_count.load(Ordering::SeqCst), 0);
}
