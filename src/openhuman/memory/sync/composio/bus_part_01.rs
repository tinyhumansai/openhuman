use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::agent::triage::{
    apply_decision, remote_trigger_origin, run_triage, TriageOutcome, TriggerEnvelope,
};
use crate::openhuman::agent::turn_origin::with_origin;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::integrations::composio::trigger_history;
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;
use tinymemory_api::host::COMPOSIO_MODE_DIRECT;

/// A backend-tenant Composio client for one toolkit.
///
/// This was `ProviderContextExt::backend_client`, an extension trait on the
/// engine's `ProviderContext` that existed for a single caller — the
/// connection-readiness probe below. With the context gone the trait had
/// nothing to extend, so the resolution lives here instead.
///
/// The config is reloaded rather than taken from the snapshot on purpose: the
/// OAuth completion this subscriber reacts to may have written credentials
/// since, and a `composio.mode` toggle mid-session must take effect on the
/// next call. Direct mode is a hard error rather than a silent fallback — the
/// helpers behind this were written against the backend tenant, and routing
/// them through the wrong one is worse than refusing (#1710).
async fn backend_composio_client(
    config: &crate::openhuman::config::Config,
    toolkit: &str,
) -> anyhow::Result<ComposioClient> {
    let live_config = crate::openhuman::config::rpc::reload_config_from_paths(
        &config.config_path,
        &config.workspace_dir,
    )
    .await
    .map_err(|e| anyhow::anyhow!("composio backend client: failed to reload live config: {e}"))?;
    match create_composio_client(&live_config)? {
        ComposioClientKind::Backend(client) => Ok(client),
        ComposioClientKind::Direct(_) => Err(anyhow::anyhow!(
            "composio direct mode is not supported on this helper path; toolkit={toolkit}"
        )),
    }
}
use crate::openhuman::integrations::composio::client::{
    create_composio_client, ComposioClient, ComposioClientKind,
};
use crate::openhuman::integrations::composio::ops;
use crate::openhuman::integrations::composio::FetchConnectedIntegrationsStatus;

/// Whether a Composio `toolkit` may be auto-registered as a memory source.
///
/// A toolkit is registrable iff a native sync provider exists for it AND the
/// bound driver can accept what it returns. A toolkit with no provider has
/// nothing that would ever call `accept_source_items` for it, so registering
/// it would report ACTIVE and then never sync — the silent lie of #4957.
/// Both auto-register sites in the connection-created handler skip
/// non-registrable toolkits. Extracted as a pure predicate so the skip
/// decision is unit-testable without driving the async event handler.
///
/// # Asked of the catalog, not of the driver
///
/// This used to ask `MemorySourceSync::is_toolkit_syncable`, a driver method
/// tinymemory v1.13.4 made unconditionally answer `Ok(false)` for every
/// toolkit — the in-process pipeline it answered from is gone, and the
/// contract kept the member rather than removing it out from under every
/// implementor. Asking it now would silently disable every Composio memory
/// source. `has_native_provider` is the answer that is still real: it names
/// the six toolkits `tinyconnectors-sync` (the module's own sync provider
/// tree) actually knows how to read — gmail, notion, slack, clickup, github,
/// linear — the same list `catalog_for_toolkit`'s callers already trust.
///
/// A binding with no `Sources` family reads as "not registrable" for the same
/// reason a driver with no provider used to: nothing here would ever be able
/// to call `accept_source_items` for it.
async fn toolkit_is_memory_source_registrable(
    config: &crate::openhuman::config::Config,
    toolkit: &str,
) -> bool {
    if !crate::openhuman::integrations::composio::providers::has_native_provider(toolkit) {
        return false;
    }
    let Ok(binding) = crate::openhuman::memory::binding::for_config(config) else {
        tracing::warn!(
            toolkit = %toolkit,
            "[composio:bus] memory binding failed; treating toolkit as not registrable"
        );
        return false;
    };
    if binding.provider().as_sources().is_none() {
        tracing::debug!(
            toolkit = %toolkit,
            driver = %binding.driver_id(),
            "[composio:bus] driver does not serve Sources; treating toolkit as not registrable"
        );
        return false;
    }
    true
}

/// Env var that **disables** the triage pipeline. The pipeline is
/// enabled by default; set to `1`/`true`/`yes` to opt out (e.g. for
/// debugging or in environments where LLM calls on every Composio
/// webhook are undesirable).
const TRIAGE_DISABLED_ENV: &str = "OPENHUMAN_TRIGGER_TRIAGE_DISABLED";

/// How long we'll keep polling the backend after `composio_authorize`
/// returns a `connectUrl`, waiting for the user to actually finish the
/// hosted OAuth flow and the connection to flip to ACTIVE/CONNECTED.
/// One minute matches typical hosted-OAuth round-trip times and is
/// generous enough to absorb a slow tab-switch + login + consent.
const CONNECTION_READY_TIMEOUT: Duration = Duration::from_secs(60);

/// Poll backoff schedule (start, max). We start aggressive so the
/// fast-path (user already had the tab open) feels immediate, then
/// back off so we don't hammer the backend during the long tail of
/// users who actually have to log in to the upstream service.
const CONNECTION_READY_INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const CONNECTION_READY_MAX_BACKOFF: Duration = Duration::from_secs(4);

static COMPOSIO_TRIGGER_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();
static COMPOSIO_CONNECTION_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();
static COMPOSIO_CONFIG_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register both long-lived composio subscribers on the global event
/// bus, and initialise the default provider registry. Idempotent.
pub fn register_composio_trigger_subscriber() {
    // No host-side provider registry any more. It existed so this process
    // could answer `get_provider`; the driver answers that question now, and
    // the module initialises its own registry in its own process
    // (tinymemory#105) — which a `cdylib`'s separate statics require anyway.

    if COMPOSIO_TRIGGER_HANDLE.get().is_none() {
        match BUS.subscribe(Arc::new(ComposioTriggerSubscriber::new())) {
            Some(handle) => {
                let _ = COMPOSIO_TRIGGER_HANDLE.set(handle);
                log::debug!("[event_bus] composio trigger subscriber registered");
            }
            None => {
                log::warn!(
                    "[event_bus] failed to register composio trigger subscriber — bus not initialized"
                );
            }
        }
    }

    if COMPOSIO_CONNECTION_HANDLE.get().is_none() {
        match BUS.subscribe(Arc::new(ComposioConnectionCreatedSubscriber::new())) {
            Some(handle) => {
                let _ = COMPOSIO_CONNECTION_HANDLE.set(handle);
                log::debug!("[event_bus] composio connection_created subscriber registered");
            }
            None => {
                log::warn!(
                    "[event_bus] failed to register composio connection_created subscriber — bus not initialized"
                );
            }
        }
    }

    if COMPOSIO_CONFIG_HANDLE.get().is_none() {
        match BUS.subscribe(Arc::new(ComposioConfigChangedSubscriber::new())) {
            Some(handle) => {
                let _ = COMPOSIO_CONFIG_HANDLE.set(handle);
                log::debug!("[event_bus] composio config_changed subscriber registered");
            }
            None => {
                log::warn!(
                    "[event_bus] failed to register composio config_changed subscriber — bus not initialized"
                );
            }
        }
    }
}

/// Logs and (when enabled) routes `ComposioTriggerReceived` events
/// through the reusable `agent::triage` pipeline.
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
        // (`wss://api.tinyhumans.ai`) which only fans out events from
        // the tinyhumans Composio tenant. When the user has switched
        // to direct mode, that tenant is no longer their active source
        // of truth — connections live on `backend.composio.dev` under
        // their own API key, and any backend-tenant triggers that keep
        // firing are ghosts from the prior mode. Drop them here so the
        // user doesn't see triage runs or history entries originating
        // from a tenant they've moved away from. Real-time triggers
        // for direct-mode users are tracked as a follow-up — see the
        // `composio.direct_mode_triggers_gap` capability and
        // `periodic.rs` docstring.
        //
        // Fail-open on config load error: if config is unreadable, we
        // let the event through rather than silently dropping it. The
        // existing env-var / config triage flags below remain the
        // backend-mode gates.
        if let Ok(config) = config_rpc::load_config_with_timeout().await {
            if config.composio.mode == COMPOSIO_MODE_DIRECT {
                tracing::info!(
                    toolkit = %toolkit,
                    trigger = %trigger,
                    "[composio:trigger] dropped — direct mode active (backend-tenant event ignored)"
                );
                return;
            }
        }

        if let Some(store) = trigger_history::global() {
            let toolkit_owned = toolkit.clone();
            let trigger_owned = trigger.clone();
            let metadata_id_owned = metadata_id.clone();
            let metadata_uuid_owned = metadata_uuid.clone();
            let payload_owned = payload.clone();

            match tokio::task::spawn_blocking(move || {
                store.record_trigger(
                    &toolkit_owned,
                    &trigger_owned,
                    &metadata_id_owned,
                    &metadata_uuid_owned,
                    &payload_owned,
                )
            })
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    tracing::warn!(
                        toolkit = %toolkit,
                        trigger = %trigger,
                        error = %error,
                        "[composio][history] failed to archive trigger"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        toolkit = %toolkit,
                        trigger = %trigger,
                        error = %error,
                        "[composio][history] failed to join archive task"
                    );
                }
            }
        } else {
            tracing::debug!(
                toolkit = %toolkit,
                trigger = %trigger,
                "[composio][history] archive store not initialized"
            );
        }

        if triage_disabled() {
            tracing::debug!(
                toolkit = %toolkit,
                trigger = %trigger,
                "[composio][triage] skipped: {TRIAGE_DISABLED_ENV} is set"
            );
            return;
        }

        // Config-level triage gates — checked after env var so the env var
        // remains a global emergency kill-switch that works even when the
        // config file is corrupt. Fail-open on load error: if we can't read
        // the config we let triage run rather than silently drop events.
        match config_rpc::load_config_with_timeout().await {
            Ok(config) => {
                if config.composio.triage_disabled {
                    tracing::debug!(
                        toolkit = %toolkit,
                        trigger = %trigger,
                        "[composio][triage] skipped: composio.triage_disabled=true in config"
                    );
                    return;
                }
                let toolkit_lower = toolkit.to_ascii_lowercase();
                if config
                    .composio
                    .triage_disabled_toolkits
                    .iter()
                    .any(|t| t.to_ascii_lowercase() == toolkit_lower)
                {
                    tracing::debug!(
                        toolkit = %toolkit,
                        trigger = %trigger,
                        "[composio][triage] skipped: toolkit in composio.triage_disabled_toolkits"
                    );
                    return;
                }
            }
            Err(e) => {
                tracing::warn!(
                    toolkit = %toolkit,
                    trigger = %trigger,
                    error = %e,
                    "[composio][triage] config load failed — falling through to triage (fail-open)"
                );
            }
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
            "[composio][triage] dispatching to agent::triage::run_triage"
        );

        // Spawn so the bus dispatch loop stays non-blocking — the
        // triage turn is an LLM round-trip that may take seconds.
        tokio::spawn(async move {
            match run_triage(&envelope).await {
                Ok(TriageOutcome::Decision(run)) => {
                    // Remote payload: a Composio trigger body is
                    // attacker-influenceable, so the dispatch parks rather than
                    // running on a trust root (#5634). Scoped here, inside the
                    // spawned task, because `AGENT_TURN_ORIGIN` is a task-local
                    // and does not cross `tokio::spawn`.
                    let origin = remote_trigger_origin(&envelope);
                    if let Err(e) = with_origin(origin, apply_decision(run, &envelope)).await {
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
                    // Tiered fallback exhausted both arms; the caller
                    // surface (composio bus) has no scheduler of its
                    // own — log and drop. The next composio fire will
                    // re-enter the chain.
                    tracing::warn!(
                        label = %envelope.display_label,
                        defer_until_ms = defer_until_ms,
                        reason = %reason,
                        "[composio][triage] run_triage deferred"
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
                    // The classifier named here is the host's own. The
                    // engine crate exposes a `report_error_or_expected` of its
                    // own, but that is a global slot for *extracted* code to
                    // report through — installed by whichever process embeds
                    // the engine, which since #5560 is the loaded TinyMemory
                    // module and no longer this one (`memory/host_impls.rs`,
                    // the host's installer, is deleted; `modules/memory_host.rs`
                    // is the bus-served twin). This subscriber is host code, so
                    // the detour buys nothing and only costs an engine
                    // dependency the seam is trying to shed.
                    crate::core::observability::report_error_or_expected(
                        detail.as_str(),
                        "composio",
                        "trigger_triage",
                        &[("label", envelope.display_label.as_str())],
                    );
                }
            }
        });
    }
}

/// Returns `true` when `OPENHUMAN_TRIGGER_TRIAGE_DISABLED` is set to a
/// truthy value. The pipeline is **on by default**; this env var is the
/// opt-out escape hatch.
fn triage_disabled() -> bool {
    matches!(
        std::env::var(TRIAGE_DISABLED_ENV).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

// ── Connection-created subscriber ───────────────────────────────────

/// Routes `ComposioConnectionCreated` events to the toolkit's provider.
pub struct ComposioConnectionCreatedSubscriber;

impl ComposioConnectionCreatedSubscriber {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ComposioConnectionCreatedSubscriber {
    fn default() -> Self {
        Self::new()
    }
}
