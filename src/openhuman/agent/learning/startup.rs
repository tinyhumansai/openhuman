//! Always-on learning subscriber wiring.
//!
//! Registers the Phase 2/3/4 learning subscribers on the global event bus:
//!
//! - **Phase 2** — the email-signature producer (reacts to
//!   `DocumentCanonicalized` events and emits Identity candidates into the
//!   learning buffer). Needs no memory client.
//! - **Phase 3** — the event-driven rebuild trigger plus the periodic 30-minute
//!   rebuild loop. Needs the global memory client.
//! - **Phase 4** — the `ProfileMdRenderer` (re-renders the five cache-derived
//!   `PROFILE.md` blocks on `CacheRebuilt`). Needs the global memory client.
//!
//! # Why this lives here (#5003)
//!
//! These three subscriptions used to be wired inside
//! `channels::runtime::startup::start_channels`. That function is a misnamed
//! process-wide bootstrap that `core::runtime::services::spawn_channels_service`
//! **skips entirely** when no chat integration is configured (or when
//! `OPENHUMAN_DISABLE_CHANNEL_LISTENERS` is set) — logging only at debug. As a
//! result, channel-less users silently got **no** learning at all.
//!
//! [`register_learning_subscribers`] is invoked from the always-on Platform
//! boot path (`core::jsonrpc::register_domain_subscribers`, the unconditional
//! `DomainGroup::Platform` block), where the memory client and workspace dir are
//! already available. Registration is idempotent, so both boot paths (and repeat
//! calls) install each subscriber exactly once.

use std::path::Path;
use std::sync::OnceLock;

use tinybus::SubscriptionHandle;

static EMAIL_SIG_HANDLE: OnceLock<Option<SubscriptionHandle>> = OnceLock::new();

/// Register the always-on learning subscribers on the global event bus.
///
/// Idempotent for any caller: every subscription is guarded by a process-wide
/// `OnceLock`, so wiring this from multiple boot paths (or calling it twice)
/// registers each subscriber exactly once. The returned `SubscriptionHandle`s
/// are intentionally leaked into statics so the subscriptions stay alive for the
/// lifetime of the process (same pattern as `TracingSubscriber`).
///
/// `workspace_dir` is the resolved workspace directory used by the
/// `ProfileMdRenderer` to locate `PROFILE.md`.
pub fn register_learning_subscribers(workspace_dir: std::path::PathBuf) {
    // Phase 2 learning producer: email-signature subscriber reacts to
    // DocumentCanonicalized events and emits Identity candidates into the
    // buffer. Needs no memory client, so it always registers.
    register_email_signature_once(&EMAIL_SIG_HANDLE, || {
        crate::openhuman::agent::learning::extract::signature::register_email_signature_subscriber()
    });

    // Phase 3 + Phase 4 learning: rebuild trigger + periodic loop + the
    // ProfileMdRenderer. All three need a bound memory driver. Readiness is
    // resolved here and the dependent work is split into `register_with_memory`
    // so both the ready and not-ready arms are unit-testable without touching
    // process globals.
    static CLIENT_HANDLES: OnceLock<(Option<SubscriptionHandle>, Option<SubscriptionHandle>)> =
        OnceLock::new();
    let memory_ready = memory_is_bindable(&workspace_dir);
    CLIENT_HANDLES.get_or_init(|| register_with_memory(memory_ready, &workspace_dir));
}

/// Whether this workspace has a usable memory driver to register learning
/// subscribers against.
///
/// This replaces `tinymemory_core::global::client_if_ready().is_some()` (#5560).
/// The two ask the same question — "is memory reachable yet?" — of different
/// things: the old call asked whether the in-process engine singleton had been
/// initialised, which after #5560 nothing does at boot, so it would have
/// answered `false` on every desktop start and silently taken learning down the
/// #5003 skip path forever.
///
/// `binding::for_workspace` is synchronous, cached, and infallible by design —
/// an inadmissible driver *falls back* rather than erroring — so the honest
/// negative here is `MemoryBinding::disables_memory`: memory explicitly
/// configured off, which is the one state in which a rebuild loop has nothing
/// to rebuild against.
fn memory_is_bindable(workspace_dir: &Path) -> bool {
    use crate::openhuman::config::schema::MemorySubsystemConfig;
    match crate::openhuman::memory::binding::for_workspace(
        workspace_dir,
        &MemorySubsystemConfig::default(),
    ) {
        Ok(binding) if binding.disables_memory() => {
            tracing::warn!(
                driver = %binding.driver_id(),
                "[learning::startup] memory is disabled for this workspace — learning subscribers will not register"
            );
            false
        }
        Ok(binding) => {
            tracing::debug!(
                driver = %binding.driver_id(),
                "[learning::startup] memory driver bound for learning subscribers"
            );
            true
        }
        Err(error) => {
            tracing::warn!("[learning::startup] no memory binding for this workspace: {error}");
            false
        }
    }
}

fn register_email_signature_once<F>(handle_cell: &OnceLock<Option<SubscriptionHandle>>, register: F)
where
    F: FnOnce() -> Option<SubscriptionHandle>,
{
    handle_cell.get_or_init(|| {
        let handle = register();
        if handle.is_some() {
            tracing::info!(
                "[learning] email-signature subscriber registered (channel-independent boot path)"
            );
        } else {
            tracing::warn!(
                "[learning] email-signature subscriber NOT registered — event bus not initialised"
            );
        }
        handle
    });
}

/// Register the client-dependent learning subscribers.
///
/// The profile facet cache for `workspace_dir`.
///
/// Resolved through the memory binding rather than the process-global client:
/// facets live behind the driver now, and `binding::for_workspace` is
/// synchronous and cached, so this stays callable from the boot path without
/// an await.
fn facet_cache_for(
    workspace_dir: &std::path::Path,
) -> Option<crate::openhuman::agent::learning::cache::FacetCache> {
    use crate::openhuman::config::schema::MemorySubsystemConfig;
    match crate::openhuman::memory::binding::for_workspace(
        workspace_dir,
        &MemorySubsystemConfig::default(),
    ) {
        Ok(binding) => Some(crate::openhuman::agent::learning::cache::FacetCache::new(
            binding.guard(),
        )),
        Err(error) => {
            tracing::warn!("[learning::startup] no memory binding for facet cache: {error}");
            None
        }
    }
}

/// Returns `(rebuild_trigger_handle, profile_md_renderer_handle)`.
///
/// When `memory_ready` is true, both the Phase 3 rebuild trigger (plus its
/// periodic 30-minute loop) and the Phase 4 `ProfileMdRenderer` are registered.
/// When it is false (this workspace has no usable memory driver) both are
/// skipped and the skip is logged at **warn** — the *silent* skip was the #5003
/// bug, so this must be loud.
///
/// Taking readiness as a parameter (rather than resolving the binding
/// internally) keeps both arms testable without touching process globals; it was
/// an `Option<MemoryClientRef>` for the same reason before #5560, and the client
/// itself was never used for anything but its presence.
fn register_with_memory(
    memory_ready: bool,
    workspace_dir: &Path,
) -> (Option<SubscriptionHandle>, Option<SubscriptionHandle>) {
    if !memory_ready {
        tracing::warn!(
            "[learning::scheduler] no memory driver for this workspace — skipping event-trigger + \
             periodic-rebuild registration; learning rebuilds will not fire (#5003)"
        );
        tracing::warn!(
            "[learning::profile_md_renderer] no memory driver for this workspace — skipping \
             ProfileMdRenderer registration; PROFILE.md will not be re-rendered (#5003)"
        );
        return (None, None);
    }

    // Phase 3 learning: event-driven rebuild trigger + periodic 30-minute loop.
    let rebuild_trigger = {
        use crate::openhuman::agent::learning::scheduler::register_event_trigger;
        use crate::openhuman::agent::learning::StabilityDetector;
        use std::sync::Arc;
        let Some(cache) = facet_cache_for(workspace_dir) else {
            return (None, None);
        };
        let detector = Arc::new(StabilityDetector::new(cache));
        // Also spawn the periodic rebuild loop (30-minute cadence).
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        // Leak the sender so the loop never receives a shutdown signal until the
        // process exits. This matches the pattern used by other always-on
        // background tasks.
        Box::leak(Box::new(shutdown_tx));
        crate::openhuman::agent::learning::scheduler::spawn_rebuild_loop(
            Arc::clone(&detector),
            crate::openhuman::agent::learning::scheduler::DEFAULT_REBUILD_INTERVAL,
            shutdown_rx,
        );
        let handle = register_event_trigger(detector);
        if handle.is_some() {
            tracing::info!(
                "[learning::scheduler] rebuild trigger + periodic loop registered \
                 (channel-independent boot path)"
            );
        }
        handle
    };

    // Phase 4 learning: ProfileMdRenderer subscribes to CacheRebuilt events and
    // re-renders the five cache-derived PROFILE.md blocks (style, identity,
    // tooling, vetoes, goals).
    let profile_md = {
        use crate::openhuman::agent::learning::ProfileMdRenderer;
        use std::sync::Arc;
        let Some(cache) = facet_cache_for(workspace_dir) else {
            return (rebuild_trigger, None);
        };
        let cache = Arc::new(cache);
        let renderer = Arc::new(ProfileMdRenderer::new(cache, workspace_dir.to_path_buf()));
        let handle = ProfileMdRenderer::subscribe(renderer);
        if handle.is_some() {
            tracing::info!(
                "[learning::profile_md_renderer] ProfileMdRenderer registered \
                 (channel-independent boot path)"
            );
        }
        handle
    };

    (rebuild_trigger, profile_md)
}

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;
