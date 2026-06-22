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

#[cfg(not(test))]
fn proactive_approval_gate() -> Option<Arc<crate::openhuman::approval::ApprovalGate>> {
    crate::openhuman::approval::ApprovalGate::try_global()
}

#[cfg(test)]
fn proactive_approval_gate() -> Option<Arc<crate::openhuman::approval::ApprovalGate>> {
    None
}

/// Register a web-only proactive message subscriber on the global event
/// bus. Guarded by `std::sync::Once` so it is safe to call from both
/// `bootstrap_core_runtime` (desktop/JSON-RPC) and domain-level
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

    /// Share this subscriber's active-channel handle so the runtime can update it
    /// in place after construction (see [`register_active_channel_handle`]).
    pub fn active_channel_handle(&self) -> Arc<RwLock<Option<String>>> {
        Arc::clone(&self.active_channel)
    }
}

/// Handle to the live proactive subscriber's `active_channel`, registered at
/// channel-runtime startup (issue #3712 — "switch default channel
/// Telegram↔Discord"). The `channels_set_default` RPC mutates this exact handle
/// via [`set_runtime_active_channel`] so a default-channel switch from the UI
/// takes effect without a restart. Only the full channel runtime registers
/// (the web-only subscriber can't deliver externally), and nothing registers in
/// unit tests — so [`set_runtime_active_channel`] is a no-op there and never
/// leaks across the parallel test suite. The choice is also persisted to
/// `config.channels_config.active_channel`, which seeds the handle on next start.
static ACTIVE_CHANNEL_HANDLE: std::sync::OnceLock<RwLock<Option<Arc<RwLock<Option<String>>>>>> =
    std::sync::OnceLock::new();

fn active_channel_handle_slot() -> &'static RwLock<Option<Arc<RwLock<Option<String>>>>> {
    ACTIVE_CHANNEL_HANDLE.get_or_init(|| RwLock::new(None))
}

/// Register the live subscriber's active-channel handle so the RPC can update it
/// at runtime. Called once from channel-runtime startup; the latest registration
/// wins.
pub fn register_active_channel_handle(handle: Arc<RwLock<Option<String>>>) {
    if let Ok(mut slot) = active_channel_handle_slot().write() {
        *slot = Some(handle);
    }
}

/// Update the live proactive subscriber's active channel. No-op when no
/// subscriber has registered a handle (e.g. unit tests, or before the channel
/// runtime starts) — config persistence still applies and the value is read at
/// next startup.
pub fn set_runtime_active_channel(channel: Option<String>) {
    // Clone the Arc out and drop the slot read-guard before locking the handle,
    // so we never hold two locks at once and the borrow doesn't outlive the read.
    let handle = match active_channel_handle_slot().read() {
        Ok(slot) => slot.clone(),
        Err(_) => return,
    };
    let Some(handle) = handle else {
        tracing::debug!("[proactive] set_runtime_active_channel: no live subscriber registered");
        return;
    };
    // Bind the guard out of the match (rather than `if let`) so the write-lock
    // temporary is dropped before `handle`, avoiding an E0597 borrow on the
    // local `handle`.
    let mut guard = match handle.write() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    tracing::debug!(channel = ?channel, "[proactive] runtime active channel updated");
    *guard = channel;
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
            error_source: None,
            error_retryable: None,
            error_retry_after_ms: None,
            error_provider: None,
            error_fallback_available: None,
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
            subagent: None,
            task_board: None,
        });

        // 2. If an active external channel is configured, deliver there too.
        //    The `channels_set_default` RPC mutates this handle in place (issue
        //    #3712), so reading it here picks up a live default-channel switch.
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
                // Resolve a delivery target before doing any work. Proactive
                // sends carry no inbound recipient, so the channel must supply
                // its configured default (e.g. Discord's `channel_id`). Channels
                // with no resolvable target (e.g. Telegram, which has no stored
                // default chat) are skipped with a warning rather than handed an
                // empty recipient that would hit the platform API with a blank
                // chat/channel id (#3794 review — Codex P2). Web delivery above
                // already happened, so skipping only drops the external echo.
                let Some(recipient) = ch.proactive_target() else {
                    tracing::warn!(
                        source = %source,
                        channel = %key,
                        "[proactive] active external channel has no configured \
                         delivery target for recipient-less proactive messages; \
                         skipping external delivery (web delivery unaffected)"
                    );
                    return;
                };

                tracing::debug!(
                    source = %source,
                    channel = %key,
                    "[proactive] delivering to active external channel"
                );

                // ── External-effect approval gate (#1339, #2135) ─
                // Proactive sends to Telegram/Discord/Slack/etc.
                // are outbound writes — route through the gate
                // before handing off to the channel implementation.
                // Web delivery above is internal and exempt. When
                // the gate persists an approval row, we keep its
                // `request_id` so we can record the delivery
                // outcome after `ch.send` returns (issue #2135).
                let mut approval_request_id: Option<String> = None;
                let mut approval_gate_for_audit: Option<
                    std::sync::Arc<crate::openhuman::approval::ApprovalGate>,
                > = None;
                if let Some(gate) = proactive_approval_gate() {
                    let summary = format!(
                        "proactive-send to {key} ({} chars)",
                        message.chars().count()
                    );
                    let redacted = serde_json::json!({
                        "channel": key,
                        "source": source.to_string(),
                        "message_chars": message.chars().count(),
                    });
                    let (outcome, request_id) = gate
                        .intercept_audited("channels.proactive_send", &summary, redacted)
                        .await;
                    match outcome {
                        crate::openhuman::approval::GateOutcome::Allow => {
                            approval_request_id = request_id;
                            if approval_request_id.is_some() {
                                approval_gate_for_audit = Some(gate);
                            }
                        }
                        crate::openhuman::approval::GateOutcome::Deny { reason } => {
                            tracing::warn!(
                                source = %source,
                                channel = %key,
                                reason = %reason,
                                "[proactive] approval gate denied external delivery"
                            );
                            return;
                        }
                    }
                }

                let send_result = ch.send(&SendMessage::new(message, &recipient)).await;
                // Record the terminal status on the approval audit
                // row before we log the outcome — best-effort, see
                // #2135. `record_execution` itself logs write
                // errors so we don't pile on here.
                if let (Some(gate), Some(req_id)) = (
                    approval_gate_for_audit.as_ref(),
                    approval_request_id.as_ref(),
                ) {
                    let (exec_outcome, err_text) = match &send_result {
                        Ok(()) => (crate::openhuman::approval::ExecutionOutcome::Success, None),
                        Err(e) => (
                            crate::openhuman::approval::ExecutionOutcome::Failure,
                            Some(e.to_string()),
                        ),
                    };
                    gate.record_execution(req_id, exec_outcome, err_text.as_deref());
                }

                match send_result {
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
                target: Some(name.to_string()),
            }
        }

        /// A channel with no resolvable proactive target (e.g. Telegram).
        fn without_target(name: &str, send_count: Arc<AtomicUsize>) -> Self {
            Self {
                name: name.to_string(),
                send_count,
                target: None,
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
        let ch: Arc<dyn Channel> = Arc::new(MockChannel::new("telegram", Arc::clone(&send_count)));
        let map: HashMap<String, Arc<dyn Channel>> = [("telegram".into(), ch)].into();
        let sub = ProactiveMessageSubscriber::new(Arc::new(map), Some("telegram".into()));

        sub.handle(&proactive_event()).await;

        assert_eq!(send_count.load(Ordering::SeqCst), 1);
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
}
