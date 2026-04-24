//! Proactive message routing.
//!
//! Subscribes to [`DomainEvent::ProactiveMessageRequested`] events and
//! delivers the message to the user's **active channel**. The active
//! channel is read from `config.channels_config.active_channel` at
//! construction time; callers can update it at runtime via
//! [`ProactiveMessageSubscriber::set_active_channel`].
//!
//! Delivery strategy:
//!
//! 1. **Web channel** — always receives the message via the Socket.IO
//!    event bus (`publish_web_channel_event`). This is the in-app
//!    experience.
//! 2. **Active external channel** — if the user has set an active
//!    channel (e.g. `"telegram"`, `"discord"`) AND that channel is in
//!    the registered channels map, the message is sent there too.
//!
//! If the active channel is `"web"` or unset, only web delivery occurs
//! (step 1). This avoids double-delivering to a channel that doesn't
//! exist.

use crate::core::event_bus::{DomainEvent, EventHandler};
use crate::core::socketio::WebChannelEvent;
use crate::openhuman::channels::providers::web::publish_web_channel_event;
use crate::openhuman::channels::{Channel, SendMessage};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Register a web-only proactive message subscriber on the global event
/// bus. Guarded by `std::sync::Once` so it is safe to call from both
/// `bootstrap_skill_runtime` (desktop/JSON-RPC) and domain-level
/// startup — only the first call takes effect.
pub fn register_web_only_proactive_subscriber() {
    use std::sync::Once;
    static REGISTERED: Once = Once::new();
    REGISTERED.call_once(|| {
        if let Some(handle) = crate::core::event_bus::subscribe_global(Arc::new(
            ProactiveMessageSubscriber::web_only(),
        )) {
            std::mem::forget(handle);
            tracing::debug!("[proactive] web-only subscriber registered");
        } else {
            tracing::warn!(
                "[proactive] failed to register web-only subscriber — bus not initialized"
            );
        }
    });
}

/// Routes proactive messages to the user's preferred channel.
pub struct ProactiveMessageSubscriber {
    /// External channels (Telegram, Discord, etc.) keyed by name.
    /// Empty in the desktop/web-only runtime.
    channels_by_name: Arc<HashMap<String, Arc<dyn Channel>>>,

    /// The user's preferred channel for proactive messages. Read from
    /// config at construction; can be updated at runtime.
    active_channel: Arc<RwLock<Option<String>>>,
}

impl ProactiveMessageSubscriber {
    /// Construct with access to the external channels map and a
    /// preferred channel name (from `channels_config.active_channel`).
    pub fn new(
        channels_by_name: Arc<HashMap<String, Arc<dyn Channel>>>,
        active_channel: Option<String>,
    ) -> Self {
        Self {
            channels_by_name,
            active_channel: Arc::new(RwLock::new(active_channel)),
        }
    }

    /// Construct a web-only subscriber (no external channels). Used in
    /// the desktop/JSON-RPC runtime where no external channel instances
    /// are registered.
    pub fn web_only() -> Self {
        Self::new(Arc::new(HashMap::new()), None)
    }

    /// Update the active channel at runtime (e.g. from an RPC call).
    pub fn set_active_channel(&self, channel: Option<String>) {
        if let Ok(mut guard) = self.active_channel.write() {
            *guard = channel;
        }
    }
}

#[async_trait]
impl EventHandler for ProactiveMessageSubscriber {
    fn name(&self) -> &str {
        "channels::proactive"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["cron"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ProactiveMessageRequested {
            source,
            message,
            job_name,
        } = event
        else {
            return;
        };

        let thread_id = format!("proactive:{}", job_name.as_deref().unwrap_or("system"));
        let request_id = uuid::Uuid::new_v4().to_string();

        tracing::debug!(
            source = %source,
            thread_id = %thread_id,
            message_len = message.len(),
            "[proactive] handling proactive message"
        );

        // 1. Always deliver to the web channel via Socket.IO.
        publish_web_channel_event(WebChannelEvent {
            event: "proactive_message".to_string(),
            client_id: "system".to_string(),
            thread_id: thread_id.clone(),
            request_id: request_id.clone(),
            full_response: Some(message.clone()),
            message: None,
            error_type: None,
            tool_name: None,
            skill_id: None,
            args: None,
            output: None,
            success: Some(true),
            round: None,
            reaction_emoji: None,
            segment_index: None,
            segment_total: None,
            delta: None,
            delta_kind: None,
            tool_call_id: None,
            citations: None,
        });

        // 2. If an active external channel is configured, deliver there too.
        let active = self
            .active_channel
            .read()
            .ok()
            .and_then(|guard| guard.clone());

        if let Some(ref channel_name) = active {
            // "web" is already handled above — skip to avoid noise.
            if channel_name.eq_ignore_ascii_case("web") {
                return;
            }

            let key = channel_name.to_ascii_lowercase();
            if let Some(ch) = self.channels_by_name.get(&key) {
                tracing::debug!(
                    source = %source,
                    channel = %key,
                    "[proactive] delivering to active external channel"
                );
                match ch.send(&SendMessage::new(message, "")).await {
                    Ok(()) => {
                        tracing::debug!(
                            source = %source,
                            channel = %key,
                            "[proactive] external delivery succeeded"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            source = %source,
                            channel = %key,
                            error = %e,
                            "[proactive] external delivery failed"
                        );
                    }
                }
            } else {
                tracing::warn!(
                    source = %source,
                    channel = %key,
                    available = ?self.channels_by_name.keys().collect::<Vec<_>>(),
                    "[proactive] active channel not found in registered channels"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::channels::traits::ChannelMessage;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::mpsc;

    struct MockChannel {
        name: String,
        send_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Channel for MockChannel {
        fn name(&self) -> &str {
            &self.name
        }
        async fn send(&self, _message: &SendMessage) -> anyhow::Result<()> {
            self.send_count.fetch_add(1, Ordering::SeqCst);
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
        let ch: Arc<dyn Channel> = Arc::new(MockChannel {
            name: "telegram".into(),
            send_count: Arc::clone(&send_count),
        });
        let map: HashMap<String, Arc<dyn Channel>> = [("telegram".into(), ch)].into();
        let sub = ProactiveMessageSubscriber::new(Arc::new(map), Some("telegram".into()));

        sub.handle(&proactive_event()).await;

        assert_eq!(send_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn skips_external_when_active_is_web() {
        let send_count = Arc::new(AtomicUsize::new(0));
        let ch: Arc<dyn Channel> = Arc::new(MockChannel {
            name: "telegram".into(),
            send_count: Arc::clone(&send_count),
        });
        let map: HashMap<String, Arc<dyn Channel>> = [("telegram".into(), ch)].into();
        let sub = ProactiveMessageSubscriber::new(Arc::new(map), Some("web".into()));

        sub.handle(&proactive_event()).await;

        // Active channel is "web" — external channel should NOT be called.
        assert_eq!(send_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn skips_external_when_active_is_none() {
        let send_count = Arc::new(AtomicUsize::new(0));
        let ch: Arc<dyn Channel> = Arc::new(MockChannel {
            name: "telegram".into(),
            send_count: Arc::clone(&send_count),
        });
        let map: HashMap<String, Arc<dyn Channel>> = [("telegram".into(), ch)].into();
        let sub = ProactiveMessageSubscriber::new(Arc::new(map), None);

        sub.handle(&proactive_event()).await;

        assert_eq!(send_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn runtime_update_active_channel() {
        let send_count = Arc::new(AtomicUsize::new(0));
        let ch: Arc<dyn Channel> = Arc::new(MockChannel {
            name: "discord".into(),
            send_count: Arc::clone(&send_count),
        });
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
        let ch: Arc<dyn Channel> = Arc::new(MockChannel {
            name: "telegram".into(),
            send_count: Arc::clone(&send_count),
        });
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
}
