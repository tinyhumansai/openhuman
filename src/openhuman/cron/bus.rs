//! Event bus handlers for the cron domain.
//!
//! When the cron scheduler needs to deliver job output to a channel (Telegram,
//! Discord, Slack, etc.), it publishes a `CronDeliveryRequested` event instead
//! of directly constructing channel instances. The [`CronDeliverySubscriber`]
//! picks up those events and dispatches to the appropriate channel, keeping
//! channel construction out of the scheduler.

use crate::openhuman::channels::{Channel, SendMessage};
use crate::openhuman::event_bus::{DomainEvent, EventHandler};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// Subscribes to `CronDeliveryRequested` events and dispatches
/// the output to the named channel.
pub struct CronDeliverySubscriber {
    channels_by_name: Arc<HashMap<String, Arc<dyn Channel>>>,
}

impl CronDeliverySubscriber {
    pub fn new(channels_by_name: Arc<HashMap<String, Arc<dyn Channel>>>) -> Self {
        Self { channels_by_name }
    }
}

#[async_trait]
impl EventHandler for CronDeliverySubscriber {
    fn name(&self) -> &str {
        "cron::delivery"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["cron"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::CronDeliveryRequested {
            job_id,
            channel,
            target,
            output,
        } = event
        else {
            return;
        };

        tracing::debug!(
            job_id = %job_id,
            channel = %channel,
            target = %target,
            output_len = output.len(),
            "[cron] handling delivery request"
        );

        let channel_lower = channel.to_ascii_lowercase();
        if let Some(ch) = self.channels_by_name.get(&channel_lower) {
            match ch.send(&SendMessage::new(output, target)).await {
                Ok(()) => {
                    tracing::debug!(
                        job_id = %job_id,
                        channel = %channel_lower,
                        "[cron] delivery succeeded"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        job_id = %job_id,
                        channel = %channel_lower,
                        error = %e,
                        "[cron] delivery failed"
                    );
                }
            }
        } else {
            tracing::warn!(
                job_id = %job_id,
                channel = %channel_lower,
                available = ?self.channels_by_name.keys().collect::<Vec<_>>(),
                "[cron] no matching channel found for delivery"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::channels::traits::ChannelMessage;
    use std::sync::atomic::{AtomicUsize, Ordering};
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
}
