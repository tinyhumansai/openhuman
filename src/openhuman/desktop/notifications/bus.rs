//! Broadcast bus + DomainEvent subscriber for core notifications.
//!
//! Mirrors the pattern used by [`overlay::bus`](crate::openhuman::desktop::overlay::bus)
//! — a single `tokio::sync::broadcast` channel wrapped in a `Lazy` static,
//! plus a [`EventHandler`] implementation that translates relevant
//! [`DomainEvent`] variants into [`CoreNotificationEvent`] payloads.
//!
//! The Socket.IO bridge in `core::socketio::spawn_web_channel_bridge`
//! subscribes to this bus and forwards every event to all connected clients
//! as `core_notification` / `core:notification` Socket.IO messages.

use once_cell::sync::Lazy;
use std::borrow::Cow;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

use crate::core::events::DomainEvent;
use crate::openhuman::config::Config;
use async_trait::async_trait;
use tinybus::EventHandler;

use super::types::{CoreNotificationCategory, CoreNotificationEvent};

const LOG_PREFIX: &str = "[core-notify]";

static NOTIFICATION_BUS: Lazy<broadcast::Sender<CoreNotificationEvent>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(128);
    tx
});

/// Subscribe to core notifications — consumed by the Socket.IO bridge at
/// startup. Additional in-process consumers (e.g. integration tests) can
/// subscribe too.
pub fn subscribe_core_notifications() -> broadcast::Receiver<CoreNotificationEvent> {
    NOTIFICATION_BUS.subscribe()
}

/// Publish a core notification. Fire-and-forget: if nobody is currently
/// subscribed the event is dropped. Returns the number of active
/// subscribers that received the event for diagnostics.
pub fn publish_core_notification(event: CoreNotificationEvent) -> usize {
    log::debug!(
        "{LOG_PREFIX} publish id={} category={:?} title_chars={}",
        event.id,
        event.category,
        event.title.len(),
    );
    NOTIFICATION_BUS.send(event).unwrap_or(0)
}

/// Subscribes to selected DomainEvent variants and translates each into a
/// [`CoreNotificationEvent`], persisting it (#3805) and broadcasting it to any
/// connected client.
///
/// `config` is `None` only in unit tests that exercise the pure translation /
/// subscriber-name contract without a workspace on disk; in production it is
/// always `Some`, so every core notification is durably stored before being
/// broadcast and therefore survives an app-closed / disconnected window.
#[derive(Default)]
pub struct NotificationBridgeSubscriber {
    config: Option<Config>,
}

impl NotificationBridgeSubscriber {
    /// Construct a subscriber that persists notifications to the workspace
    /// store backed by `config` before broadcasting them.
    pub fn new(config: Config) -> Self {
        Self {
            config: Some(config),
        }
    }

    /// The workspace store a notification belongs in.
    ///
    /// Normally this bridge's own: it is registered once, with the workspace
    /// that booted, and every event it handled before #5931 is process-wide.
    ///
    /// The MCP supervisor variants are the exception. One process supervises
    /// every workspace it has opened (`mcp::host::all_hosts`), and a workspace
    /// switch leaves the old one open and still supervised, so a supervisor
    /// event can belong to a workspace that is not this bridge's — including
    /// the *active* one, once the boot binding is the stale half of the pair.
    /// Neither answer to that is right on its own: filing it under this
    /// bridge's workspace puts one account's outage in another's inbox, and
    /// dropping it silences the workspace the user is actually in. So the
    /// event says where it belongs and nothing is dropped — the binding's
    /// freshness stops mattering.
    ///
    /// Only `workspace_dir` is redirected because it is the only field the
    /// notification store reads: `store::with_connection` opens
    /// `<workspace_dir>/notifications/notifications.db` and looks at nothing
    /// else. Broadcast is unaffected — `NOTIFICATION_BUS` is process-wide and
    /// always has been.
    fn store_target<'a>(&self, config: &'a Config, event: &DomainEvent) -> Cow<'a, Config> {
        match workspace_of(event) {
            Some(event_workspace) if event_workspace != config.workspace_dir.as_path() => {
                log::debug!(
                    "{LOG_PREFIX} filing {} under its own workspace event_ws={} self_ws={}",
                    event.variant_name(),
                    event_workspace.display(),
                    config.workspace_dir.display()
                );
                let mut redirected = config.clone();
                redirected.workspace_dir = event_workspace.to_path_buf();
                Cow::Owned(redirected)
            }
            _ => Cow::Borrowed(config),
        }
    }

    /// Whether a notification should reach connected clients.
    ///
    /// Storing an event under its own workspace is only half the answer. The
    /// live path has no per-client routing at all — `core::socketio`'s bridge
    /// emits `core_notification` to *every* connected client, and the banner
    /// prints the server's qualified name and its error — so a supervisor
    /// event from a workspace the user has switched away from would show one
    /// account's server, and its failure text, inside the account they are
    /// actually in (#5931).
    ///
    /// Only workspace-bound events are gated, and the active workspace is
    /// resolved through `config::active_workspace_dir`, the same resolver the
    /// config loader uses — deliberately not a value pinned at construction,
    /// which is exactly what goes stale across a switch. It costs one small
    /// marker read, paid only for the handful of events a supervisor tick
    /// produces: everything else returns on the first line.
    ///
    /// **Fails closed.** If the workspace cannot be resolved, nothing is
    /// announced. That costs a *banner*, not the alert: the notification is
    /// already persisted under its own workspace by the time this runs, so it
    /// still reaches the notification centre — which is exactly the durability
    /// split #3805 built, with the store as the reliable channel and the
    /// broadcast as best effort. Announcing on an unknown workspace would
    /// instead put another account's server name and transport error in front
    /// of whoever is connected, and there is no undoing that.
    async fn should_announce(&self, event: &DomainEvent) -> Announce {
        let Some(event_workspace) = workspace_of(event) else {
            return Announce::Unbound;
        };
        // One snapshot, so the revision stamped on the notification is the
        // one the comparison below was made against. Resolved separately, a
        // switch in between would stamp an event from workspace A with B's
        // newer revision — and a client that had not yet seen the switch
        // would read that as "I am behind" and accept the stale alert.
        let snapshot = match crate::openhuman::config::active_workspace_snapshot().await {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                log::warn!(
                    "{LOG_PREFIX} could not resolve the active workspace ({error}); not announcing {} — it is persisted and will show in the notification centre",
                    event.variant_name()
                );
                None
            }
        };
        let active = snapshot.as_ref().map(|(dir, _)| dir.as_path());
        let announces = announces_to(Some(event_workspace), active);
        if !announces && active.is_some() {
            log::debug!(
                "{LOG_PREFIX} not announcing {} from an inactive workspace event_ws={} active_ws={}",
                event.variant_name(),
                event_workspace.display(),
                active.unwrap_or(std::path::Path::new("?")).display()
            );
        }
        match (announces, snapshot) {
            (true, Some((_, revision))) => Announce::Active(revision),
            _ => Announce::Suppressed,
        }
    }
}

/// The announcement gate's verdict.
///
/// Carries the revision the gate compared against so the caller can stamp
/// exactly that on the payload — a receiver whose own revision is older then
/// knows it is simply behind on the switch broadcast, rather than looking at
/// a stale notification (#5966).
enum Announce {
    /// Not workspace-bound: announced everywhere, nothing to stamp.
    Unbound,
    /// Bound to the active workspace: announced, stamped with the revision
    /// that workspace was current under when the gate checked.
    Active(u64),
    /// Bound to a workspace that is not active, or the active workspace could
    /// not be resolved (fail closed — see `should_announce`).
    Suppressed,
}

/// The announcement rule, as a function of its two inputs.
///
/// Separated from the I/O so the decision can be asserted directly — in
/// particular the fail-closed arm, which is otherwise reachable only by making
/// the on-disk config unreadable.
///
/// - `event_workspace` `None`: the event is not workspace-bound, so it is not
///   this rule's business and is announced.
/// - `active` `None`: the active workspace could not be resolved. **Fail
///   closed** — see [`NotificationBridgeSubscriber::should_announce`].
/// - Otherwise the event is announced only in the workspace it belongs to.
fn announces_to(
    event_workspace: Option<&std::path::Path>,
    active: Option<&std::path::Path>,
) -> bool {
    match (event_workspace, active) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(event), Some(active)) => event == active,
    }
}

/// The workspace an event belongs to, for the variants that name one.
///
/// `None` means the event is not bound to a workspace and any subscriber may
/// act on it.
///
/// This used to list the MCP supervisor variants here, which was correct for
/// what the bridge handled at the time but wrong as a general rule: the
/// channel and artifact families carry the same field and were not matched,
/// so a bridge that grew an arm for one of them would have gated it against
/// nothing. The list now lives on the event itself (#5966), where a new
/// workspace-bound variant is one arm to add rather than several to find.
fn workspace_of(event: &DomainEvent) -> Option<&std::path::Path> {
    event.workspace_dir()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Pure translation function — kept free so unit tests can drive it
/// without spinning up tokio or the broadcast channel.
///
/// The workspace handle is stamped here rather than inside [`translate`],
/// once, from the event's own
/// [`workspace_dir`](DomainEvent::workspace_dir). Letting each arm decide
/// would reproduce the shape of the bug #5966 exists to fix: the arms that
/// happened to be written with a workspace in mind would carry the identity
/// and the rest would silently not, and which is which would depend on the
/// order the arms were added.
pub fn event_to_notification(event: &DomainEvent) -> Option<CoreNotificationEvent> {
    let mut notification = translate(event)?;
    notification.workspace = event
        .workspace_dir()
        .map(crate::openhuman::config::workspace_handle);
    Some(notification)
}

/// The per-variant translation. Every arm leaves `workspace: None`; the
/// identity is applied uniformly by [`event_to_notification`], which is the
/// only caller.
fn translate(event: &DomainEvent) -> Option<CoreNotificationEvent> {
    let ts = now_ms();
    match event {
        DomainEvent::CronJobCompleted {
            job_id, success, ..
        } => Some(CoreNotificationEvent {
            id: format!("cron:{}:{}", job_id, ts),
            category: CoreNotificationCategory::Agents,
            title: if *success {
                "Cron job completed".into()
            } else {
                "Cron job failed".into()
            },
            body: if *success {
                format!("Job {job_id} finished successfully.")
            } else {
                format!("Job {job_id} did not complete — check your cron schedule.")
            },
            deep_link: Some("/settings/cron-jobs".into()),
            timestamp_ms: ts,
            actions: None,
            workspace: None,
            workspace_revision: None,
        }),
        DomainEvent::WebhookProcessed {
            skill_id,
            status_code,
            elapsed_ms,
            error,
            ..
        } => {
            // Only surface failures — successful webhooks are noisy.
            if error.is_none() && *status_code < 400 {
                return None;
            }
            Some(CoreNotificationEvent {
                id: format!("webhook:{}:{}", skill_id, ts),
                category: CoreNotificationCategory::System,
                title: "Webhook error".into(),
                body: match error {
                    Some(err) => {
                        format!("{skill_id} webhook failed after {elapsed_ms}ms: {err}")
                    }
                    None => format!(
                        "{skill_id} webhook returned HTTP {status_code} after {elapsed_ms}ms"
                    ),
                },
                deep_link: Some("/settings/webhooks-triggers".into()),
                timestamp_ms: ts,
                actions: None,
                workspace: None,
                workspace_revision: None,
            })
        }
        DomainEvent::SubagentCompleted {
            parent_session,
            task_id,
            agent_id,
            output_chars,
            ..
        } => Some(CoreNotificationEvent {
            id: format!("subagent:{}:{}:{}", parent_session, task_id, ts),
            category: CoreNotificationCategory::Agents,
            title: "Sub-agent finished".into(),
            body: format!("{agent_id} produced {output_chars} chars of output."),
            deep_link: Some("/chat".into()),
            timestamp_ms: ts,
            actions: None,
            workspace: None,
            workspace_revision: None,
        }),
        DomainEvent::SubagentFailed {
            parent_session,
            task_id,
            agent_id,
            error,
        } => Some(CoreNotificationEvent {
            id: format!("subagent:{}:{}:{}", parent_session, task_id, ts),
            category: CoreNotificationCategory::Agents,
            title: "Sub-agent failed".into(),
            body: format!(
                "{agent_id} encountered an error: {}",
                error.chars().take(100).collect::<String>()
            ),
            deep_link: Some("/chat".into()),
            timestamp_ms: ts,
            actions: None,
            workspace: None,
            workspace_revision: None,
        }),
        DomainEvent::NotificationTriaged {
            id,
            provider,
            action,
            importance_score,
            latency_ms,
            routed,
        } if *routed && (action == "escalate" || action == "react") => {
            Some(CoreNotificationEvent {
                id: format!("notification-triaged:{}:{}:{}", id, action, latency_ms),
                category: CoreNotificationCategory::Agents,
                title: format!("High-priority {} notification", provider),
                body: if action == "escalate" {
                    format!(
                        "Action: escalate (score: {:.0}%). Routed to orchestrator.",
                        importance_score * 100.0
                    )
                } else {
                    format!(
                        "Action: react (score: {:.0}%). Routed for follow-up.",
                        importance_score * 100.0
                    )
                },
                deep_link: Some("/notifications".into()),
                timestamp_ms: ts,
                actions: None,
                workspace: None,
                workspace_revision: None,
            })
        }
        DomainEvent::ProviderApiKeyRejected { provider, message } => Some(CoreNotificationEvent {
            id: format!("provider-key-rejected:{}:{}", provider, ts),
            category: CoreNotificationCategory::System,
            title: "API key rejected".into(),
            // `message` is already a pre-formatted, actionable string from
            // `auth_error_registry::auth_error_message`.
            body: message.clone(),
            // Land the user on the AI-settings LLM tab, where the inline
            // provider-error notice + key editor live. Must be the canonical
            // `/connections?tab=llm` route: `/skills` is a back-compat
            // redirect that drops the query and defaults to the Apps tab, so
            // it would not surface the key editor.
            deep_link: Some("/connections?tab=llm".into()),
            timestamp_ms: ts,
            actions: None,
            workspace: None,
            workspace_revision: None,
        }),
        // The MCP reconnect supervisor's verdicts (#5931), published by
        // `mcp::registry::supervisor_events`. Only the cases a user can act on
        // or would otherwise wonder about become notifications: a server that
        // *stays* down after a tick, its recovery, and a server the supervisor
        // has given up on. A session torn down and rebuilt within one tick, or
        // a single slow probe, is Event Log material only — nothing was
        // unavailable long enough for anyone to notice, and a banner per blip
        // would be the fourteen-a-day noise this exists to replace.
        DomainEvent::McpServerReconnectFailed {
            server_id,
            qualified_name,
            error,
            failures,
            ..
        } if *failures == 1 => Some(CoreNotificationEvent {
            id: format!("mcp-unavailable:{}:{}", server_id, ts),
            category: CoreNotificationCategory::System,
            title: "MCP server unavailable".into(),
            // Deliberately no "retrying in Ns": `retry_in_secs` is tinymcp's
            // backoff (5s for the first failure) and is faithful on the event,
            // but this host drives `Supervisor::tick` from its own 60s
            // interval, so the backoff only decides *eligibility* on the next
            // tick. Every sub-tick step (5/10/20/40s) would be under-reported
            // to the user, who then watches a "5s" banner sit there for a
            // minute. The exact backoff is still in the Event Log row via
            // `DomainEvent::log_detail`, where a developer can read it against
            // the tick interval; a banner cannot carry that caveat.
            body: format!(
                "{qualified_name} stopped answering, so its tools are unavailable until it \
                 reconnects. It retries automatically. {}",
                crate::core::events::clip_to_chars(error, 120)
            ),
            deep_link: Some("/connections?tab=mcp".into()),
            timestamp_ms: ts,
            actions: None,
            workspace: None,
            workspace_revision: None,
        }),
        DomainEvent::McpServerReconnected {
            server_id,
            qualified_name,
            tool_count,
            after_failures,
            ..
        } if *after_failures > 0 => Some(CoreNotificationEvent {
            id: format!("mcp-restored:{}:{}", server_id, ts),
            category: CoreNotificationCategory::System,
            title: "MCP server reconnected".into(),
            body: format!(
                "{qualified_name} is back with {tool_count} tools after {after_failures} failed \
                 attempt(s)."
            ),
            deep_link: Some("/connections?tab=mcp".into()),
            timestamp_ms: ts,
            actions: None,
            workspace: None,
            workspace_revision: None,
        }),
        DomainEvent::McpServerParked {
            server_id,
            qualified_name,
            error,
            ..
        } => Some(CoreNotificationEvent {
            id: format!("mcp-parked:{}:{}", server_id, ts),
            category: CoreNotificationCategory::System,
            title: "MCP server can't start".into(),
            body: format!(
                "{qualified_name} will not be retried: {}. Install the missing runtime, then \
                 disable and re-enable the server.",
                crate::core::events::clip_to_chars(error, 160)
            ),
            deep_link: Some("/connections?tab=mcp".into()),
            timestamp_ms: ts,
            actions: None,
            workspace: None,
            workspace_revision: None,
        }),
        _ => None,
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for NotificationBridgeSubscriber {
    fn name(&self) -> &str {
        "notifications::bridge"
    }

    // `domains()` returns None — we filter at the variant match instead of
    // the domain string, since we pull from three different domains and
    // the domain list is an optional short-circuit rather than a
    // correctness boundary.

    async fn handle(&self, event: &DomainEvent) {
        if let Some(mut notification) = event_to_notification(event) {
            // #3805: persist BEFORE broadcasting so the event is durable even
            // when no client is currently subscribed (app closed / minimised /
            // disconnected) — otherwise the broadcast send finds zero
            // receivers and the notification is lost forever. Best-effort: a
            // store failure must not suppress the live broadcast.
            if let Some(config) = &self.config {
                // A workspace-bound event is stored in ITS OWN workspace, not
                // whichever one this bridge was registered with (#5931).
                let config = self.store_target(config, event);
                match super::store::insert_core_notification(&config, &notification) {
                    Ok(true) => log::debug!(
                        "{LOG_PREFIX} persisted core notification id={}",
                        notification.id
                    ),
                    Ok(false) => log::debug!(
                        "{LOG_PREFIX} core notification id={} already persisted (dedup)",
                        notification.id
                    ),
                    Err(err) => log::warn!(
                        "{LOG_PREFIX} failed to persist core notification id={}: {err}",
                        notification.id
                    ),
                }
            }
            // Persisted above under the workspace it belongs to; announced
            // only if that workspace is the one the user is in (#5931).
            match self.should_announce(event).await {
                Announce::Unbound => {
                    publish_core_notification(notification);
                }
                Announce::Active(revision) => {
                    notification.workspace_revision = Some(revision);
                    publish_core_notification(notification);
                }
                Announce::Suppressed => {}
            };
        }
    }
}

/// Register the notification bridge subscriber on the global event bus.
/// Safe to call multiple times — each call produces a fresh subscription,
/// but the caller (`register_domain_subscribers`) is Once-guarded.
pub fn register_notification_bridge_subscriber(config: Config) {
    use std::sync::Arc;
    if let Some(handle) =
        crate::core::bus::BUS.subscribe(Arc::new(NotificationBridgeSubscriber::new(config)))
    {
        // SAFETY: intentional leak; handle's Drop would cancel the subscriber.
        std::mem::forget(handle);
        log::info!("{LOG_PREFIX} notification bridge subscriber registered");
    } else {
        log::warn!(
            "{LOG_PREFIX} failed to register notification bridge — event bus not initialized"
        );
    }
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod bus_tests;

#[cfg(test)]
#[path = "bus_tests_2_tests.rs"]
mod tests;
