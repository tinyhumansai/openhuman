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
//! - **Active external channel** — if the user has set an active
//!    channel (e.g. `"telegram"`, `"discord"`) AND that channel is in
//!    the registered channels map, the message is sent there too.
//! - The independent `web_chat::proactive` subscriber always delivers the web
//!   destination, including when the external channels feature is disabled.

use crate::channels::runtime::ChannelHandle;
use crate::channels::SendMessage;
use crate::core::events::DomainEvent;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tinybus::EventHandler;

#[cfg(not(test))]
fn proactive_approval_gate() -> Option<Arc<crate::security::approval::ApprovalGate>> {
    crate::security::approval::ApprovalGate::try_global()
}

#[cfg(test)]
fn proactive_approval_gate() -> Option<Arc<crate::security::approval::ApprovalGate>> {
    None
}

/// Routes proactive messages to the user's preferred channel.
pub(crate) struct ProactiveMessageSubscriber {
    /// External channels (Telegram, Discord, etc.) keyed by name.
    /// Empty in the desktop/web-only runtime.
    channels_by_name: Arc<HashMap<String, Arc<ChannelHandle>>>,

    /// The user's preferred channel for proactive messages. Read from
    /// config at construction; can be updated at runtime.
    active_channel: Arc<RwLock<Option<String>>>,
}

impl ProactiveMessageSubscriber {
    /// Construct with access to the external channels map and a
    /// preferred channel name (from `channels_config.active_channel`).
    pub(crate) fn new(
        channels_by_name: Arc<HashMap<String, Arc<ChannelHandle>>>,
        active_channel: Option<String>,
    ) -> Self {
        Self {
            channels_by_name,
            active_channel: Arc::new(RwLock::new(active_channel)),
        }
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
/// and nothing registers in unit tests — so [`set_runtime_active_channel`] is a no-op there and never
/// leaks across the parallel test suite. The choice is also persisted to
/// `config.channels_config.active_channel`, which seeds the handle on next start.
type ActiveChannelHandle = Arc<RwLock<Option<String>>>;
type ActiveChannelHandleSlot = RwLock<Option<ActiveChannelHandle>>;

static ACTIVE_CHANNEL_HANDLE: std::sync::OnceLock<ActiveChannelHandleSlot> =
    std::sync::OnceLock::new();

fn active_channel_handle_slot() -> &'static ActiveChannelHandleSlot {
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
impl EventHandler<DomainEvent> for ProactiveMessageSubscriber {
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
            ..
        } = event
        else {
            return;
        };

        let thread_id = format!("proactive:{}", job_name.as_deref().unwrap_or("system"));
        tracing::debug!(
            source = %source,
            thread_id = %thread_id,
            message_len = message.len(),
            "[proactive] handling proactive message"
        );

        // Deliver to the active external channel when configured. The web
        // destination is handled independently by `web_chat::proactive`. The
        // `channels_set_default` RPC mutates this handle in place (issue #3712),
        // so reading it here picks up a live default-channel switch.
        let active = self
            .active_channel
            .read()
            .ok()
            .and_then(|guard| guard.clone());

        if let Some(ref channel_name) = active {
            // "web" is handled by `web_chat::proactive` — skip to avoid noise.
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
                // chat/channel id (#3794 review — Codex P2).
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
                // The web subscriber is an internal destination and exempt.
                // When the gate persists an approval row, we keep its
                // request id so we can record the delivery
                // outcome after `ch.send` returns (issue #2135).
                let mut approval_request_id: Option<String> = None;
                let mut approval_gate_for_audit: Option<
                    std::sync::Arc<crate::security::approval::ApprovalGate>,
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
                    // This runs on a spawned bus task, where the publisher's
                    // task-local turn origin is gone and the gate would deny it
                    // as `Unknown`. The event is host-originated automation
                    // (a cron job's output, a briefing), so label it as such.
                    // This bus task delivers a result to a destination the
                    // operator configured. It is distinct from the cron
                    // agent turn that produced the result, whose tool calls
                    // must remain subject to the cron effect restriction.
                    let automation_source =
                        crate::security::turn_origin::TrustedAutomationSource::Background;
                    let (outcome, request_id) = crate::security::turn_origin::with_origin(
                        crate::security::turn_origin::AgentTurnOrigin::TrustedAutomation {
                            job_id: source.clone(),
                            source: automation_source,
                        },
                        gate.intercept_audited(
                            &Default::default(),
                            "channels.proactive_send",
                            &summary,
                            redacted,
                        ),
                    )
                    .await;
                    match outcome {
                        crate::security::approval::GateOutcome::Allow => {
                            approval_request_id = request_id;
                            if approval_request_id.is_some() {
                                approval_gate_for_audit = Some(gate);
                            }
                        }
                        crate::security::approval::GateOutcome::Deny { reason } => {
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

                let send_result = ch
                    .send_with_outbound_intent(&SendMessage::new(message, &recipient))
                    .await;
                // Record the terminal status on the approval audit
                // row before we log the outcome — best-effort, see
                // #2135. `record_execution` itself logs write
                // errors so we don't pile on here.
                if let (Some(gate), Some(req_id)) = (
                    approval_gate_for_audit.as_ref(),
                    approval_request_id.as_ref(),
                ) {
                    let (exec_outcome, err_text) = match &send_result {
                        Ok(()) => (crate::security::approval::ExecutionOutcome::Success, None),
                        Err(e) => (
                            crate::security::approval::ExecutionOutcome::Failure,
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
#[path = "proactive_tests.rs"]
mod tests;
