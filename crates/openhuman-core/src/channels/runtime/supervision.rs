//! OpenHuman observer adapter for TinyChannels listener supervision.

use super::super::Channel;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use std::sync::Arc;
use tinychannels::runtime::ListenerObserver;
use tinychannels_bus as traits;

/// Publishes reusable listener lifecycle facts onto OpenHuman's domain bus.
struct OpenHumanListenerObserver;

impl ListenerObserver for OpenHumanListenerObserver {
    fn connected(&self, channel: &str) {
        BUS.publish(DomainEvent::ChannelConnected {
            channel: channel.to_string(),
        });
    }

    fn disconnected(&self, channel: &str, reason: &str, failed: bool) {
        if failed {
            let message = format!("Channel {channel} error: {reason}; restarting");
            crate::core::observability::report_error_or_expected(
                &message,
                "channels",
                "supervised_listener",
                &[("channel", channel)],
            );
        } else {
            tracing::warn!("Channel {channel} exited unexpectedly; restarting");
        }
        BUS.publish(DomainEvent::ChannelDisconnected {
            channel: channel.to_string(),
            reason: reason.to_string(),
        });
    }

    fn restarted(&self, channel: &str) {
        BUS.publish(DomainEvent::HealthRestarted {
            component: format!("channel:{channel}"),
        });
    }
}

// Adapt host-owned test doubles to the legacy provider supervisor. Production
// receives supervision facts from the compiled module instead.
struct LegacyListenerFixture(Arc<dyn Channel>);

#[async_trait::async_trait]
impl tinychannels::Channel for LegacyListenerFixture {
    fn name(&self) -> &str {
        self.0.name()
    }

    async fn send(&self, message: &traits::SendMessage) -> anyhow::Result<()> {
        self.0.send(message).await
    }

    async fn listen(
        &self,
        tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
    ) -> anyhow::Result<()> {
        self.0.listen(tx).await
    }

    async fn health_check(&self) -> bool {
        self.0.health_check().await
    }
}

/// Start a legacy listener fixture with OpenHuman observability attached.
pub(crate) fn spawn_supervised_listener(
    channel: Arc<dyn Channel>,
    tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
    initial_backoff_secs: u64,
    max_backoff_secs: u64,
) -> tokio::task::JoinHandle<()> {
    crate::platform::health::bus::register_health_subscriber();
    tinychannels::runtime::spawn_supervised_listener(
        Arc::new(LegacyListenerFixture(channel)),
        tx,
        initial_backoff_secs,
        max_backoff_secs,
        Arc::new(OpenHumanListenerObserver),
    )
}

#[cfg(test)]
pub(crate) use tinychannels::runtime::{jitter_millis, MAX_JITTER_MS};

#[cfg(test)]
#[path = "supervision_tests.rs"]
mod tests;
