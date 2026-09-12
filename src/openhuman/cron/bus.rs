//! Event bus handlers for the cron domain.
//!
//! When the cron scheduler needs to deliver job output to a channel (Telegram,
//! Discord, Slack, etc.), it publishes a `CronDeliveryRequested` event instead
//! of directly constructing channel instances. The [`CronDeliverySubscriber`]
//! picks up those events and dispatches to the appropriate channel, keeping
//! channel construction out of the scheduler.

use crate::core::events::DomainEvent;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tinybus::EventHandler;
use tinychannels_bus::{Channel, SendMessage};

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
impl EventHandler<DomainEvent> for CronDeliverySubscriber {
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
                    crate::core::observability::report_error(
                        &e,
                        "cron",
                        "delivery",
                        &[
                            ("job_id", job_id.as_str()),
                            ("channel", channel_lower.as_str()),
                            ("failure", "channel_send"),
                        ],
                    );
                }
            }
        } else {
            let msg = format!(
                "no matching channel '{}' found for cron delivery (job {})",
                channel_lower, job_id
            );
            crate::core::observability::report_error(
                msg.as_str(),
                "cron",
                "delivery",
                &[
                    ("job_id", job_id.as_str()),
                    ("channel", channel_lower.as_str()),
                    ("failure", "channel_missing"),
                ],
            );
        }
    }
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
