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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::event_bus::{init_global, publish_global, DomainEvent, DEFAULT_CAPACITY};
    use crate::openhuman::learning::candidate::{self, EvidenceRef};
    use crate::openhuman::learning::extract::signature::parse_signature;
    use crate::openhuman::memory_store::MemoryClient;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Build a real `MemoryClient` against a fresh temp workspace. The temp dir
    /// is returned so callers keep it alive for the client's lifetime.
    fn test_client() -> (TempDir, MemoryClientRef) {
        let tmp = TempDir::new().expect("tempdir");
        let client = Arc::new(
            MemoryClient::from_workspace_dir(tmp.path().join("workspace"))
                .expect("client should initialise against a fresh workspace"),
        );
        (tmp, client)
    }

    /// Process-unique email source id so buffer assertions never collide with
    /// candidates pushed by other tests running in parallel against the shared
    /// global buffer.
    fn unique_source_id(tag: &str) -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        format!(
            "gmail:5003-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// A body whose trailing lines form a clear email signature — yields several
    /// Identity candidates (name/role/timezone/employer).
    fn signature_body() -> String {
        "Hi, great to hear from you!\n\n\
         Thanks,\n\
         Alice Johnson\n\
         Senior Software Engineer\n\
         Acme Corp\n\
         San Francisco, CA\n\
         PST"
        .to_string()
    }

    fn publish_email_doc(source_id: &str, body: &str) {
        publish_global(DomainEvent::DocumentCanonicalized {
            source_id: source_id.to_string(),
            source_kind: "email".to_string(),
            chunks_written: 1,
            chunk_ids: vec![format!("{source_id}-c1")],
            canonicalized_at: 0.0,
            body_preview: Some(body.to_string()),
        });
    }

    /// Count candidates in the global buffer whose evidence points at
    /// `source_id`. Isolates this test's assertions from concurrent producers.
    fn candidates_for(source_id: &str) -> usize {
        candidate::global()
            .peek()
            .iter()
            .filter(|c| {
                matches!(
                    &c.evidence,
                    EvidenceRef::EmailMessage { source_id: sid, .. } if sid == source_id
                )
            })
            .count()
    }

    /// Poll the global buffer until at least `expected` candidates for
    /// `source_id` appear (async bus delivery), then settle briefly and return
    /// the final count so an accidental double-registration would surface.
    async fn wait_for_candidates(source_id: &str, expected: usize) -> usize {
        for _ in 0..200 {
            if candidates_for(source_id) >= expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // Settle: let any (unexpected) duplicate subscriber also deliver.
        tokio::time::sleep(Duration::from_millis(40)).await;
        candidates_for(source_id)
    }

    #[tokio::test]
    async fn register_with_client_registers_both_handles_when_ready() {
        init_global(DEFAULT_CAPACITY);
        let (tmp, client) = test_client();
        let (trigger, renderer) = register_with_client(Some(client), tmp.path());
        assert!(
            trigger.is_some(),
            "rebuild trigger must register when the memory client is ready"
        );
        assert!(
            renderer.is_some(),
            "ProfileMdRenderer must register when the memory client is ready"
        );
    }

    #[tokio::test]
    async fn register_with_client_skips_and_warns_when_client_absent() {
        // No memory client → both client-dependent subscribers are skipped and
        // the (now loud) warn path is exercised. This is the else-arm the #5003
        // fix upgraded from a silent debug-level skip.
        let tmp = TempDir::new().expect("tempdir");
        let (trigger, renderer) = register_with_client(None, tmp.path());
        assert!(trigger.is_none(), "no trigger without a client");
        assert!(renderer.is_none(), "no renderer without a client");
    }

    #[tokio::test]
    async fn learning_subscriber_fires_with_no_channel_configured() {
        init_global(DEFAULT_CAPACITY);
        let (tmp, _client) = test_client();
        // Make the memory client ready so the full Platform wiring runs — no
        // channel runtime is ever constructed in this test.
        let _ = crate::openhuman::memory::global::init(tmp.path().join("workspace"));
        register_learning_subscribers(tmp.path().to_path_buf());

        let source_id = unique_source_id("e2e");
        let body = signature_body();
        let expected = parse_signature(&body, &source_id, &source_id).len();
        assert!(
            expected > 0,
            "signature body must yield at least one identity candidate"
        );

        publish_email_doc(&source_id, &body);
        let got = wait_for_candidates(&source_id, expected).await;
        assert_eq!(
            got, expected,
            "email-signature subscriber must push the parsed identity candidates \
             with no channel configured anywhere (#5003)"
        );
    }

    #[tokio::test]
    async fn register_learning_subscribers_is_idempotent() {
        init_global(DEFAULT_CAPACITY);
        let tmp = TempDir::new().expect("tempdir");
        let _ = crate::openhuman::memory::global::init(tmp.path().join("workspace"));
        // Call twice — the process-wide OnceLock guards must keep exactly one
        // email-signature subscriber alive, so a single event is handled once.
        register_learning_subscribers(tmp.path().to_path_buf());
        register_learning_subscribers(tmp.path().to_path_buf());

        let source_id = unique_source_id("idem");
        let body = signature_body();
        let expected = parse_signature(&body, &source_id, &source_id).len();
        assert!(expected > 0);

        publish_email_doc(&source_id, &body);
        let got = wait_for_candidates(&source_id, expected).await;
        assert_eq!(
            got, expected,
            "double registration must not double the pushed candidates (#5003 idempotency)"
        );
    }
}
