//! Provider-specific code for Composio toolkits.
//!
//! Each Composio toolkit (gmail, notion, slack, …) can register a
//! [`ComposioProvider`] implementation that knows how to:
//!
//!   * Fetch a normalized **user profile** for a connected account.
//!   * Run an **initial / periodic sync** that pulls fresh data from the
//!     upstream service via the backend-proxied
//!     [`ComposioClient`](super::client::ComposioClient).
//!   * React to **trigger webhooks** that arrive over the
//!     `composio:trigger` Socket.IO bridge.
//!   * React to **OAuth handoff completion** so the very first sync can
//!     run as soon as a user connects an account.
//!
//! Providers are pure Rust — there is no JS sandbox involved. They are
//! the native counterpart to the QuickJS skill bundles in
//! `tinyhumansai/openhuman-skills`, but specialized for Composio's API
//! surface and run inside the core process directly.
//!
//! ## Registry & dispatch
//!
//! The [`registry`] module owns a process-global `HashMap<toolkit_slug,
//! Arc<dyn ComposioProvider>>`. The composio event bus subscriber
//! ([`super::bus::ComposioTriggerSubscriber`]) and the periodic sync
//! task both look up providers by toolkit slug and call into them.
//!
//! ## Why a trait, not a giant `match`
//!
//! Each provider has provider-specific shapes (gmail returns
//! emailAddress + messagesTotal, notion returns workspaces + pages, …)
//! and a different idea of what "sync" means. A trait keeps each
//! provider's implementation isolated, individually testable, and
//! easy to add without touching the dispatch layer.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::openhuman::config::Config;

use super::client::{build_composio_client, ComposioClient};

pub mod gmail;
pub mod notion;
pub mod profile;
pub mod registry;
pub mod sync_state;

pub use registry::{
    all_providers, get_provider, init_default_providers, register_provider, ProviderArc,
};

/// Reason a sync was triggered. Providers can use this to decide
/// whether to do a full backfill or an incremental pull.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncReason {
    /// First sync immediately after an OAuth handoff completes.
    ConnectionCreated,
    /// Periodic background sync from the scheduler.
    Periodic,
    /// Explicit user-driven sync from RPC / UI.
    Manual,
}

impl SyncReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            SyncReason::ConnectionCreated => "connection_created",
            SyncReason::Periodic => "periodic",
            SyncReason::Manual => "manual",
        }
    }
}

/// Normalized user profile shape returned by every provider.
///
/// The shared fields (`display_name`, `email`, `username`, `avatar_url`)
/// cover what the desktop UI actually needs to render a connected
/// account card. Anything provider-specific (Gmail's `messagesTotal`,
/// Notion's workspace ids, …) goes into [`extras`](Self::extras) so
/// callers don't have to widen the shape every time a new toolkit
/// lands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderUserProfile {
    pub toolkit: String,
    pub connection_id: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    /// Provider-specific extras (raw JSON object).
    #[serde(default)]
    pub extras: serde_json::Value,
}

/// Result of a provider sync run. Mostly used for logging + UI status.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncOutcome {
    pub toolkit: String,
    pub connection_id: Option<String>,
    pub reason: String,
    pub items_ingested: usize,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub summary: String,
    /// Provider-specific extras (raw JSON object).
    #[serde(default)]
    pub details: serde_json::Value,
}

impl SyncOutcome {
    pub fn elapsed_ms(&self) -> u64 {
        self.finished_at_ms.saturating_sub(self.started_at_ms)
    }
}

/// Per-call context handed to provider methods.
///
/// `connection_id` is `None` when a method runs in a "no specific
/// connection" mode (e.g. an across-the-board periodic sync that
/// already iterated). For per-connection paths it is always populated.
#[derive(Clone)]
pub struct ProviderContext {
    pub config: Arc<Config>,
    pub client: ComposioClient,
    pub toolkit: String,
    pub connection_id: Option<String>,
}

impl ProviderContext {
    /// Build a context from the current config + a toolkit slug.
    ///
    /// Returns `None` if a [`ComposioClient`] cannot be constructed
    /// (no JWT yet — user not signed in). Callers should treat that
    /// case as "skip silently" rather than as a hard error, mirroring
    /// the existing op layer.
    pub fn from_config(
        config: Arc<Config>,
        toolkit: impl Into<String>,
        connection_id: Option<String>,
    ) -> Option<Self> {
        let client = build_composio_client(&config)?;
        Some(Self {
            config,
            client,
            toolkit: toolkit.into(),
            connection_id,
        })
    }

    /// Memory client handle if the global memory singleton is ready.
    /// Used by providers that want to persist sync snapshots.
    pub fn memory_client(&self) -> Option<crate::openhuman::memory::MemoryClientRef> {
        crate::openhuman::memory::global::client_if_ready()
    }
}

/// Native provider implementation for a specific Composio toolkit.
///
/// All methods are async and return `Result<_, String>` so the bus
/// subscriber + RPC layer can forward errors as user-visible strings
/// without `anyhow` round-tripping.
#[async_trait]
pub trait ComposioProvider: Send + Sync {
    /// Toolkit slug (e.g. `"gmail"`). Must match the slug Composio /
    /// the backend allowlist uses — the registry keys on this.
    fn toolkit_slug(&self) -> &'static str;

    /// Suggested periodic sync interval in seconds. Return `None` to
    /// opt out of the periodic scheduler entirely (e.g. for write-only
    /// providers like Slack send-message).
    fn sync_interval_secs(&self) -> Option<u64> {
        Some(15 * 60)
    }

    /// Fetch a normalized user profile for the current connection in
    /// `ctx`. Most providers implement this by calling a provider
    /// "get profile / about me" action via [`super::ops::composio_execute`].
    async fn fetch_user_profile(
        &self,
        ctx: &ProviderContext,
    ) -> Result<ProviderUserProfile, String>;

    /// Run a sync pass for the current connection in `ctx`. Implementations
    /// are responsible for persisting whatever they fetch (typically into
    /// the memory layer via [`ProviderContext::memory_client`]).
    async fn sync(&self, ctx: &ProviderContext, reason: SyncReason) -> Result<SyncOutcome, String>;

    /// Hook fired when an OAuth handoff completes
    /// ([`crate::core::event_bus::DomainEvent::ComposioConnectionCreated`]).
    ///
    /// Default impl: fetch the user profile, then run an initial sync.
    /// Providers can override to add provider-specific bootstrapping
    /// (e.g. registering Composio triggers, seeding labels, …).
    async fn on_connection_created(&self, ctx: &ProviderContext) -> Result<(), String> {
        let toolkit = self.toolkit_slug();
        tracing::info!(
            toolkit = %toolkit,
            connection_id = ?ctx.connection_id,
            "[composio:provider] on_connection_created → fetch_user_profile + initial sync"
        );
        match self.fetch_user_profile(ctx).await {
            Ok(profile) => {
                // PII discipline: do not log raw display_name or email.
                // We log only presence indicators and the email domain
                // (non-PII) so the trace is debuggable without leaking
                // the user's identity. Provider-specific impls follow
                // the same convention.
                let has_display_name = profile.display_name.is_some();
                let has_email = profile.email.is_some();
                let email_domain = profile
                    .email
                    .as_deref()
                    .and_then(|e| e.split('@').nth(1))
                    .map(|d| d.to_string());
                tracing::info!(
                    toolkit = %toolkit,
                    has_display_name,
                    has_email,
                    email_domain = ?email_domain,
                    "[composio:provider] user profile fetched"
                );

                // Persist profile fields into the local user_profile
                // facet table so display_name / email / avatar are
                // available to the agent context and UI without a
                // round-trip to the upstream provider.
                let facets = profile::persist_provider_profile(&profile);
                tracing::debug!(
                    toolkit = %toolkit,
                    facets_written = facets,
                    "[composio:provider] profile facets persisted"
                );
            }
            Err(e) => {
                tracing::warn!(
                    toolkit = %toolkit,
                    error = %e,
                    "[composio:provider] user profile fetch failed (continuing to sync)"
                );
            }
        }
        let outcome = self.sync(ctx, SyncReason::ConnectionCreated).await?;
        tracing::info!(
            toolkit = %toolkit,
            items = outcome.items_ingested,
            elapsed_ms = outcome.elapsed_ms(),
            "[composio:provider] initial sync complete"
        );
        Ok(())
    }

    /// Hook fired when a Composio trigger webhook arrives for this
    /// toolkit. `payload` is the raw provider payload as forwarded by
    /// the backend. Implementations should be defensive — payload
    /// shapes vary across triggers.
    ///
    /// Default impl: log and no-op. Most providers will want to
    /// override this to react to specific triggers.
    async fn on_trigger(
        &self,
        ctx: &ProviderContext,
        trigger: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        tracing::debug!(
            toolkit = %self.toolkit_slug(),
            trigger = %trigger,
            connection_id = ?ctx.connection_id,
            payload_bytes = payload.to_string().len(),
            "[composio:provider] on_trigger (default no-op)"
        );
        Ok(())
    }
}

/// Helper used by every provider's `fetch_user_profile` impl.
///
/// Walks a JSON object using a list of dotted-path candidates and
/// returns the first non-empty string match. Keeps each provider's
/// extraction code free of repetitive `as_object().and_then(...)`
/// chains.
pub(crate) fn pick_str(value: &serde_json::Value, paths: &[&str]) -> Option<String> {
    for path in paths {
        let mut cur = value;
        let mut ok = true;
        for segment in path.split('.') {
            match cur.get(segment) {
                Some(next) => cur = next,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        if let Some(s) = cur.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pick_str_finds_first_non_empty_match() {
        let v = json!({
            "data": { "user": { "email": "  user@example.com  ", "name": "" } },
            "fallback": "fallback@example.com"
        });
        // first path empty -> falls through
        assert_eq!(
            pick_str(&v, &["data.user.name", "data.user.email"]),
            Some("user@example.com".to_string())
        );
        // missing path -> falls through to fallback
        assert_eq!(
            pick_str(&v, &["data.missing", "fallback"]),
            Some("fallback@example.com".to_string())
        );
        // nothing matches
        assert_eq!(pick_str(&v, &["nope.nope"]), None);
    }

    #[test]
    fn sync_outcome_elapsed_ms_is_safe_when_finish_lt_start() {
        let mut o = SyncOutcome::default();
        o.started_at_ms = 100;
        o.finished_at_ms = 50;
        assert_eq!(o.elapsed_ms(), 0);
        o.finished_at_ms = 250;
        assert_eq!(o.elapsed_ms(), 150);
    }
}
