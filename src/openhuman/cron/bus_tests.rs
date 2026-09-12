use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tinychannels_bus::ChannelMessage;
use tokio::sync::mpsc;

/// Minimal mock channel that tracks send() calls.
struct MockChannel {
    name: String,
    send_count: Arc<AtomicUsize>,
    fail: bool,
}

#[async_trait]
impl Channel for MockChannel {
    fn name(&self) -> &str {
        &self.name
    }
    async fn send(&self, _message: &SendMessage) -> anyhow::Result<()> {
        self.send_count.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            anyhow::bail!("mock send failure");
        }
        Ok(())
    }
    async fn listen(&self, _tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        Ok(())
    }
}

fn delivery_event(channel: &str) -> DomainEvent {
    DomainEvent::CronDeliveryRequested {
        job_id: "test-job".into(),
        channel: channel.into(),
        target: "chat-123".into(),
        output: "hello".into(),
    }
}

fn make_subscriber(channels: Vec<Arc<dyn Channel>>) -> CronDeliverySubscriber {
    let map: HashMap<String, Arc<dyn Channel>> = channels
        .into_iter()
        .map(|c| (c.name().to_string(), c))
        .collect();
    CronDeliverySubscriber::new(Arc::new(map))
}

#[tokio::test]
async fn ignores_non_delivery_events() {
    let send_count = Arc::new(AtomicUsize::new(0));
    let ch: Arc<dyn Channel> = Arc::new(MockChannel {
        name: "telegram".into(),
        send_count: Arc::clone(&send_count),
        fail: false,
    });
    let sub = make_subscriber(vec![ch]);

    sub.handle(&DomainEvent::CronJobTriggered {
        job_id: "j1".into(),
        job_name: "test-job".into(),
        job_type: "shell".into(),
    })
    .await;

    assert_eq!(send_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn dispatches_to_matching_channel() {
    let send_count = Arc::new(AtomicUsize::new(0));
    let ch: Arc<dyn Channel> = Arc::new(MockChannel {
        name: "telegram".into(),
        send_count: Arc::clone(&send_count),
        fail: false,
    });
    let sub = make_subscriber(vec![ch]);

    sub.handle(&delivery_event("Telegram")).await;

    assert_eq!(send_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn missing_channel_does_not_panic() {
    let sub = make_subscriber(vec![]);
    // Should log a warning but not panic.
    sub.handle(&delivery_event("nonexistent")).await;
}

#[tokio::test]
async fn send_failure_does_not_panic() {
    let send_count = Arc::new(AtomicUsize::new(0));
    let ch: Arc<dyn Channel> = Arc::new(MockChannel {
        name: "slack".into(),
        send_count: Arc::clone(&send_count),
        fail: true,
    });
    let sub = make_subscriber(vec![ch]);

    // Should log a warning but not panic.
    sub.handle(&delivery_event("slack")).await;
    assert_eq!(send_count.load(Ordering::SeqCst), 1);
}
