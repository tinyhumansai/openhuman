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

use crate::core::event_bus::SubscriptionHandle;
use crate::openhuman::memory::global::client_if_ready;
use crate::openhuman::memory_store::MemoryClientRef;

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
    static EMAIL_SIG_HANDLE: OnceLock<Option<SubscriptionHandle>> = OnceLock::new();
    EMAIL_SIG_HANDLE.get_or_init(|| {
        let handle =
            crate::openhuman::learning::extract::signature::register_email_signature_subscriber();
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

    // Phase 3 + Phase 4 learning: rebuild trigger + periodic loop + the
    // ProfileMdRenderer. All three need the global memory client. The
    // client-dependent work is split into `register_with_client` so both the
    // ready and not-ready arms are unit-testable without touching process
    // globals.
    static CLIENT_HANDLES: OnceLock<(Option<SubscriptionHandle>, Option<SubscriptionHandle>)> =
        OnceLock::new();
    CLIENT_HANDLES.get_or_init(|| register_with_client(client_if_ready(), &workspace_dir));
}

/// Register the client-dependent learning subscribers.
///
/// Returns `(rebuild_trigger_handle, profile_md_renderer_handle)`.
///
/// When `client` is `Some`, both the Phase 3 rebuild trigger (plus its periodic
/// 30-minute loop) and the Phase 4 `ProfileMdRenderer` are registered. When
/// `client` is `None` (the memory client is not yet initialised) both are
/// skipped and the skip is logged at **warn** — the *silent* skip was the #5003
/// bug, so this must be loud.
///
/// Taking the client as a parameter (rather than reading
/// `memory::global::client_if_ready()` internally) keeps both arms testable
/// without initialising the process-global memory singleton.
fn register_with_client(
    client: Option<MemoryClientRef>,
    workspace_dir: &Path,
) -> (Option<SubscriptionHandle>, Option<SubscriptionHandle>) {
    let Some(client) = client else {
        tracing::warn!(
            "[learning::scheduler] memory client not ready at boot — skipping event-trigger + \
             periodic-rebuild registration; learning rebuilds will not fire until the client \
             initialises (#5003)"
        );
        tracing::warn!(
            "[learning::profile_md_renderer] memory client not ready at boot — skipping \
             ProfileMdRenderer registration; PROFILE.md will not be re-rendered until the client \
             initialises (#5003)"
        );
        return (None, None);
    };

    // Phase 3 learning: event-driven rebuild trigger + periodic 30-minute loop.
    let rebuild_trigger = {
        use crate::openhuman::learning::cache::FacetCache;
        use crate::openhuman::learning::scheduler::register_event_trigger;
        use crate::openhuman::learning::StabilityDetector;
        use std::sync::Arc;
        let cache = FacetCache::new(client.profile_conn());
        let detector = Arc::new(StabilityDetector::new(cache));
        // Also spawn the periodic rebuild loop (30-minute cadence).
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        // Leak the sender so the loop never receives a shutdown signal until the
        // process exits. This matches the pattern used by other always-on
        // background tasks.
        Box::leak(Box::new(shutdown_tx));
        crate::openhuman::learning::scheduler::spawn_rebuild_loop(
            Arc::clone(&detector),
            crate::openhuman::learning::scheduler::DEFAULT_REBUILD_INTERVAL,
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
        use crate::openhuman::learning::cache::FacetCache;
        use crate::openhuman::learning::ProfileMdRenderer;
        use std::sync::Arc;
        let cache = Arc::new(FacetCache::new(client.profile_conn()));
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
