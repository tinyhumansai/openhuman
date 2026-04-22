//! The core provider trait for Composio toolkit implementations.

use async_trait::async_trait;

use super::tool_scope::CuratedTool;
use super::types::{ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason};

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

    /// Curated whitelist of Composio actions this provider considers
    /// useful for the agent, classified by [`super::tool_scope::ToolScope`].
    ///
    /// When `Some(&[...])`, the meta-tool layer hides every action not
    /// in this list from `composio_list_tools` and rejects execution of
    /// any slug not in this list (or whose scope is disabled in the
    /// user's pref).
    ///
    /// Default: `None` — toolkits without a curated catalog (e.g.
    /// integrations not yet hand-tuned) pass through all actions and
    /// rely on the [`super::tool_scope::classify_unknown`] heuristic for
    /// scope gating.
    fn curated_tools(&self) -> Option<&'static [CuratedTool]> {
        None
    }

    /// Fetch a normalized user profile for the current connection in
    /// `ctx`. Most providers implement this by calling a provider
    /// "get profile / about me" action via [`super::super::ops::composio_execute`].
    async fn fetch_user_profile(
        &self,
        ctx: &ProviderContext,
    ) -> Result<ProviderUserProfile, String>;

    /// Run a sync pass for the current connection in `ctx`. Implementations
    /// are responsible for persisting whatever they fetch (typically into
    /// the memory layer via [`ProviderContext::memory_client`]).
    async fn sync(&self, ctx: &ProviderContext, reason: SyncReason) -> Result<SyncOutcome, String>;

    /// Standardized identity callback for provider implementations.
    ///
    /// Providers can override this to customize how identity fragments
    /// are persisted. Default behavior stores a normalized identity
    /// fragment in profile facets via `skill:{source}:{identifier}:{field}`
    /// keys and returns the number of facets written.
    fn identity_set(&self, profile: &ProviderUserProfile) -> usize {
        super::profile::persist_provider_profile(profile)
    }

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
                let facets = self.identity_set(&profile);
                tracing::debug!(
                    toolkit = %toolkit,
                    facets_written = facets,
                    "[composio:provider] identity_set persisted profile facets"
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

    /// Hook fired immediately after a Composio action executed against
    /// this toolkit returns a **successful** response. The provider may
    /// mutate `data` in place to reshape the upstream payload before it
    /// is handed back to the agent / RPC caller (e.g. convert Gmail's
    /// HTML message bodies to markdown to save context tokens).
    ///
    /// `slug` is the full action slug (e.g. `"GMAIL_FETCH_EMAILS"`) so
    /// providers can dispatch per action. `arguments` is the caller's
    /// original argument object — providers can read opt-out flags from
    /// it (e.g. `raw_html: true` to preserve raw HTML).
    ///
    /// Errors from upstream are not routed here; only `successful`
    /// responses. Default impl is a no-op so providers that have nothing
    /// to rewrite don't need to override.
    fn post_process_action_result(
        &self,
        slug: &str,
        arguments: Option<&serde_json::Value>,
        data: &mut serde_json::Value,
    ) {
        let _ = (slug, arguments, data);
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
