//! Supervisor helpers for channel listeners.

use super::super::context::{
    CHANNEL_MAX_IN_FLIGHT_MESSAGES, CHANNEL_MIN_IN_FLIGHT_MESSAGES, CHANNEL_PARALLELISM_PER_CHANNEL,
};
use super::super::traits;
use super::super::Channel;
use crate::core::event_bus::{publish_global, DomainEvent};
use std::sync::Arc;
use std::time::Duration;

pub(crate) fn spawn_supervised_listener(
    ch: Arc<dyn Channel>,
    tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
    initial_backoff_secs: u64,
    max_backoff_secs: u64,
) -> tokio::task::JoinHandle<()> {
    // This helper is used directly in tests and isolated runtime paths, so make
    // sure channel health events always have a live bus + subscriber target.
    crate::core::event_bus::init_global(crate::core::event_bus::DEFAULT_CAPACITY);
    crate::openhuman::health::bus::register_health_subscriber();

    tokio::spawn(async move {
        let component = format!("channel:{}", ch.name());
        let mut backoff = initial_backoff_secs.max(1);
        let max_backoff = max_backoff_secs.max(backoff);

        tracing::info!(
            channel = ch.name(),
            initial_backoff_secs,
            max_backoff_secs,
            "[channels] supervised listener started"
        );

        loop {
            publish_global(DomainEvent::ChannelConnected {
                channel: ch.name().to_string(),
            });
            tracing::debug!(
                channel = ch.name(),
                "[channels] listener entering recv loop"
            );
            let result = ch.listen(tx.clone()).await;

            if tx.is_closed() {
                break;
            }

            match result {
                Ok(()) => {
                    tracing::warn!("Channel {} exited unexpectedly; restarting", ch.name());
                    publish_global(DomainEvent::ChannelDisconnected {
                        channel: ch.name().to_string(),
                        reason: "exited unexpectedly".to_string(),
                    });
                    // Clean exit — reset backoff since the listener ran successfully
                    backoff = initial_backoff_secs.max(1);
                }
                Err(e) => {
                    tracing::error!("Channel {} error: {e}; restarting", ch.name());
                    publish_global(DomainEvent::ChannelDisconnected {
                        channel: ch.name().to_string(),
                        reason: e.to_string(),
                    });
                }
            }

            publish_global(DomainEvent::HealthRestarted {
                component: component.clone(),
            });
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            // Double backoff AFTER sleeping so first error uses initial_backoff
            backoff = backoff.saturating_mul(2).min(max_backoff);
        }
    })
}

pub(crate) fn compute_max_in_flight_messages(channel_count: usize) -> usize {
    channel_count
        .saturating_mul(CHANNEL_PARALLELISM_PER_CHANNEL)
        .clamp(
            CHANNEL_MIN_IN_FLIGHT_MESSAGES,
            CHANNEL_MAX_IN_FLIGHT_MESSAGES,
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_max_in_flight_messages_zero_channels() {
        let result = compute_max_in_flight_messages(0);
        assert_eq!(result, CHANNEL_MIN_IN_FLIGHT_MESSAGES);
    }

    #[test]
    fn compute_max_in_flight_messages_one_channel() {
        let result = compute_max_in_flight_messages(1);
        assert!(result >= CHANNEL_MIN_IN_FLIGHT_MESSAGES);
        assert!(result <= CHANNEL_MAX_IN_FLIGHT_MESSAGES);
    }

    #[test]
    fn compute_max_in_flight_messages_many_channels() {
        let result = compute_max_in_flight_messages(100);
        assert_eq!(result, CHANNEL_MAX_IN_FLIGHT_MESSAGES);
    }

    #[test]
    fn compute_max_in_flight_messages_clamps_to_min() {
        let result = compute_max_in_flight_messages(0);
        assert!(result >= CHANNEL_MIN_IN_FLIGHT_MESSAGES);
    }

    #[test]
    fn compute_max_in_flight_messages_clamps_to_max() {
        let result = compute_max_in_flight_messages(usize::MAX);
        assert!(result <= CHANNEL_MAX_IN_FLIGHT_MESSAGES);
    }
}
