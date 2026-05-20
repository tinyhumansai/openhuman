//! Event-bus subscriber for Telegram remote-control lifecycle signals.

use crate::core::event_bus::{DomainEvent, EventHandler};
use crate::openhuman::channels::providers::telegram::session_store::with_store;
use async_trait::async_trait;
use std::path::PathBuf;

const LOG_PREFIX: &str = "[telegram-remote]";

/// Tracks Telegram turn lifecycle via channel domain events and exposes busy
/// state for `/status`.
pub struct TelegramRemoteSubscriber {
    workspace_dir: PathBuf,
}

impl TelegramRemoteSubscriber {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }

    async fn set_busy(&self, reply_target: &str, busy: bool) {
        let workspace_dir = self.workspace_dir.clone();
        let reply_target_owned = reply_target.to_string();
        let join_result = tokio::task::spawn_blocking(move || {
            with_store(&workspace_dir, |store| {
                store.set_busy(&reply_target_owned, busy);
                Ok(())
            })
        })
        .await;

        match join_result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(
                "{LOG_PREFIX} failed to persist busy={busy} reply_target={reply_target}: {error}"
            ),
            Err(error) => tracing::warn!(
                "{LOG_PREFIX} join error persisting busy={busy} reply_target={reply_target}: {error}"
            ),
        }
    }
}

#[async_trait]
impl EventHandler for TelegramRemoteSubscriber {
    fn name(&self) -> &str {
        "telegram::remote_control"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["channel"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::ChannelMessageReceived {
                channel,
                reply_target,
                ..
            } if channel == "telegram" => {
                tracing::debug!("{LOG_PREFIX} turn started reply_target={reply_target}");
                self.set_busy(reply_target, true).await;
            }
            DomainEvent::ChannelMessageProcessed {
                channel,
                reply_target,
                success,
                elapsed_ms,
                ..
            } if channel == "telegram" => {
                tracing::debug!(
                    "{LOG_PREFIX} turn finished reply_target={reply_target} success={success} elapsed_ms={elapsed_ms}"
                );
                self.set_busy(reply_target, false).await;
            }
            _ => {}
        }
    }
}
