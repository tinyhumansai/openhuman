//! Logs and (when enabled) triages `ComposioTriggerReceived` events.

use crate::core::runtime::DispatchCtx;
use async_trait::async_trait;
use tinybus::EventHandler;

use crate::core::events::DomainEvent;
use crate::integrations::composio::trigger_history;
use std::sync::Arc;

use crate::triage::{RemoteTriage, TriageOutcome, TriageRunner, TriggerEnvelope};

/// Env var that **disables** the triage pipeline. The pipeline is
/// enabled by default; set to `1`/`true`/`yes` to opt out (e.g. for
/// debugging or in environments where LLM calls on every Composio
/// webhook are undesirable).
pub(super) const TRIAGE_DISABLED_ENV: &str = "OPENHUMAN_TRIGGER_TRIAGE_DISABLED";

/// Logs and (when enabled) routes `ComposioTriggerReceived` events
/// through the reusable `triage` pipeline.
pub struct ComposioTriggerSubscriber;

impl ComposioTriggerSubscriber {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ComposioTriggerSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for ComposioTriggerSubscriber {
    fn name(&self) -> &str {
        "composio::trigger"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["composio"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ComposioTriggerReceived {
            toolkit,
            trigger,
            metadata_id,
            metadata_uuid,
            payload,
        } = event
        else {
            return;
        };

        tracing::debug!(
            toolkit = %toolkit,
            trigger = %trigger,
            id = %metadata_id,
            uuid = %metadata_uuid,
            payload_bytes = payload.to_string().len(),
            "[composio:bus] trigger received"
        );

        // [composio-direct] Direct-mode trigger gate.
        //
        // Inbound `composio:trigger` events ride the backend socket
        // (the managed backend socket) which only fans out events from
        // the tinyhumans Composio tenant. When the user has switched
        // to direct mode, that tenant is no longer their active source
        // of truth — connections live on `backend.composio.dev` under
        // their own API key, and any backend-tenant triggers that keep
        // firing are ghosts from the prior mode. Drop them here so the
        // user doesn't see triage runs or history entries originating
        // from a tenant they've moved away from. Real-time triggers
        // for direct-mode users are tracked as a follow-up — see the
        // `composio.direct_mode_triggers_gap` capability.
        //
        // Fail-open on config load error: if config is unreadable, we
        // let the event through rather than silently dropping it. The
        // existing env-var / config triage flags below remain the
        // backend-mode gates.
        // A trigger arrives over the host's socket, so it belongs to the host's
        // own tenant; in SaaS it has none and is dropped.
        let ctx = DispatchCtx::host_default();
        if let Ok(config) = ctx.load_config().await {
            if is_direct_mode(&config) {
                tracing::info!(
                    toolkit = %toolkit,
                    trigger = %trigger,
                    "[composio:trigger] dropped — direct mode active (backend-tenant event ignored)"
                );
                return;
            }
        }

        if let Some(store) = trigger_history::global() {
            match ctx.load_config().await {
                Ok(config) => {
                    let event = super::super::types::ComposioTriggerEvent {
                        toolkit: toolkit.clone(),
                        trigger: trigger.clone(),
                        payload: payload.clone(),
                        metadata: super::super::types::ComposioTriggerMetadata {
                            id: metadata_id.clone(),
                            uuid: metadata_uuid.clone(),
                            ..Default::default()
                        },
                    };
                    if let Err(error) = store.record(&ctx, &config, event).await {
                        tracing::warn!(error = %error, "[composio][history] failed to archive trigger");
                    }
                }
                Err(error) => {
                    tracing::warn!(error = %error, "[composio][history] archive configuration unavailable")
                }
            }
        }

        // The env var is a global kill-switch that works even when the config
        // file is corrupt, so it is checked before (and instead of) a config
        // read; an unreadable config otherwise fails open to triage.
        let env_disabled = triage_disabled();
        let config = if env_disabled {
            None
        } else {
            Some(ctx.load_config().await)
        };
        let readable = config.as_ref().and_then(|config| config.as_ref().ok());
        if let Some(reason) = triage_skip_reason(readable, toolkit, env_disabled) {
            tracing::debug!(
                toolkit = %toolkit,
                trigger = %trigger,
                "[composio][triage] skipped: {reason}"
            );
            return;
        }
        if let Some(Err(e)) = &config {
            tracing::warn!(
                toolkit = %toolkit,
                trigger = %trigger,
                error = %e,
                "[composio][triage] config load failed — falling through to triage (fail-open)"
            );
        }

        // Build the envelope outside the spawned task so any panic in
        // `from_composio` surfaces on the bus dispatch thread (where
        // the broadcast subscriber loop can log it) rather than being
        // swallowed inside a detached task.
        let envelope = TriggerEnvelope::from_composio(
            toolkit,
            trigger,
            metadata_id,
            metadata_uuid,
            payload.clone(),
        );
        tracing::debug!(
            label = %envelope.display_label,
            external_id = %envelope.external_id,
            "[composio][triage] dispatching to triage::run_triage"
        );

        // With the inbound event queues on, the trigger is queued: the module
        // persists it, rejects a redelivery of the same trigger, paces a
        // flood, and calls the host back (`Deliver`) to triage it, retrying a
        // deferred triage then. Off or unavailable, triage runs directly.
        // Its own task: the hand-off may wait for the module's first load and
        // retries an unanswered ingest, which must never hold up this
        // subscriber's loop (a stalled loop lags and loses broadcast events).
        let spawner = ctx.clone();
        spawner.spawn(async move {
            let config = ctx.load_config().await.ok();
            route_trigger(
                ctx,
                config,
                envelope,
                Arc::new(RemoteTriage),
                #[cfg(feature = "inbound-queues")]
                None,
            )
            .await;
        });
    }
}

/// Where a trigger went.
#[derive(Debug)]
pub(super) enum Routed {
    /// The inbound event queues own it.
    Queued,
    /// Triage runs directly, in this task.
    Direct(tokio::task::JoinHandle<()>),
}

/// Hand `envelope` to the inbound event queues, or triage it directly.
///
/// `client` reaches the queues module (`None`: the configured module
/// client). The direct path is the one triggers always took: triage in its
/// own task, a decision applied under the remote-trigger origin.
pub(super) async fn route_trigger(
    ctx: DispatchCtx,
    config: Option<crate::config::Config>,
    envelope: TriggerEnvelope,
    runner: Arc<dyn TriageRunner>,
    #[cfg(feature = "inbound-queues")] client: Option<tinybus::module::ModuleClient>,
) -> Routed {
    #[cfg(feature = "inbound-queues")]
    if let Some(config) = config.as_ref() {
        match crate::inbound::events::composio(&envelope) {
            Ok(event) => {
                let client = client.unwrap_or_else(|| crate::modules::ops::client(config.clone()));
                let offered = crate::inbound::route::offer(config, &client, event).await;
                if offered.is_queued() {
                    tracing::debug!(
                        label = %envelope.display_label,
                        "[composio][triage] handed to the inbound event queues"
                    );
                    return Routed::Queued;
                }
            }
            Err(why) => tracing::info!(
                label = %envelope.display_label,
                reason = %why,
                "[composio][triage] not queued; triaging directly"
            ),
        }
    }
    #[cfg(not(feature = "inbound-queues"))]
    let _ = config;
    Routed::Direct(triage_directly(ctx, envelope, runner))
}

/// Triage `envelope` in its own task and apply a decision: the direct path.
pub(super) fn triage_directly(
    ctx: DispatchCtx,
    envelope: TriggerEnvelope,
    runner: Arc<dyn TriageRunner>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        match runner.run(&ctx, &envelope).await {
            Ok(TriageOutcome::Decision(run)) => {
                // Remote payload: a Composio trigger body is
                // attacker-influenceable, so the runner applies the decision
                // under the remote-trigger origin (#5634), inside this task.
                if let Err(e) = runner.apply(&ctx, run, &envelope).await {
                    tracing::error!(
                        label = %envelope.display_label,
                        error = %e,
                        "[composio][triage] apply_decision failed"
                    );
                }
            }
            Ok(TriageOutcome::Deferred {
                defer_until_ms,
                reason,
            }) => {
                // Tiered fallback exhausted both arms. The direct path has
                // no scheduler of its own, so the deferral is logged and
                // dropped; with the inbound event queues on (the default)
                // triggers do not come here: `Deliver` retries a deferral
                // at `defer_until_ms` (crate::inbound::deliver).
                tracing::warn!(
                    label = %envelope.display_label,
                    defer_until_ms = defer_until_ms,
                    reason = %reason,
                    "[composio][triage] run_triage deferred"
                );
            }
            Ok(TriageOutcome::Terminal { reason }) => {
                tracing::warn!(
                    label = %envelope.display_label,
                    reason = %reason,
                    "[composio][triage] run_triage reached terminal state"
                );
            }
            Err(e) => {
                // Route through the central observability classifier
                // so user-config / budget-exhausted / provider-state
                // rollups from `reliable.rs` (e.g. `The model
                // \`<id>\` may not be available on your provider …`)
                // get demoted to info-level breadcrumbs instead of
                // surfacing as raw Sentry errors. Previously this
                // call used `tracing::error!` directly and bypassed
                // the classifier — 10.7k events / 14d on self-hosted
                // Sentry TAURI-RUST-1V, dominated by
                // ProviderConfigRejection-class rollups whose inner
                // attempts the provider layer already demoted.
                let detail = format!(
                    "[composio][triage] run_triage failed (label={}): {e:#}",
                    envelope.display_label
                );
                crate::core::observability::report_error_or_expected(
                    detail.as_str(),
                    "composio",
                    "trigger_triage",
                    &[("label", envelope.display_label.as_str())],
                );
            }
        }
    })
}

/// Backend-tenant triggers are ghosts once the user has switched to direct mode.
pub(super) fn is_direct_mode(config: &crate::config::Config) -> bool {
    config.composio.mode == crate::config::schema::COMPOSIO_MODE_DIRECT
}

/// Why a received trigger skips triage, if it does. The env kill-switch wins,
/// then the config's global switch and per-toolkit list (case-insensitive).
/// `None` config means it could not be read, which fails open to triage.
pub(super) fn triage_skip_reason(
    config: Option<&crate::config::Config>,
    toolkit: &str,
    env_disabled: bool,
) -> Option<&'static str> {
    if env_disabled {
        return Some("OPENHUMAN_TRIGGER_TRIAGE_DISABLED is set");
    }
    let composio = &config?.composio;
    if composio.triage_disabled {
        return Some("composio.triage_disabled=true in config");
    }
    composio
        .triage_disabled_toolkits
        .iter()
        .any(|t| t.eq_ignore_ascii_case(toolkit))
        .then_some("toolkit in composio.triage_disabled_toolkits")
}

/// Returns `true` when `OPENHUMAN_TRIGGER_TRIAGE_DISABLED` is set to a
/// truthy value. The pipeline is **on by default**; this env var is the
/// opt-out escape hatch.
pub(super) fn triage_disabled() -> bool {
    matches!(
        std::env::var(TRIAGE_DISABLED_ENV).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

// The tests drive the linked `tinyeventqueues` module (`inbound::fixture`).
#[cfg(all(test, feature = "inbound-queues"))]
#[path = "trigger_subscriber_tests.rs"]
mod tests;
