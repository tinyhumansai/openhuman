//! The core provider trait for Composio toolkit implementations.

use async_trait::async_trait;

use super::tool_scope::CuratedTool;
use super::types::{
    NormalizedTask, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason, TaskContainer,
    TaskFetchFilter,
};

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

    /// Fetch a filtered set of work items as structured
    /// [`NormalizedTask`]s — the read path that powers the
    /// `task_sources` domain.
    ///
    /// Unlike [`Self::sync`], this does **not** persist anything into
    /// the memory store; it *returns* normalized tasks so the caller can
    /// enrich them and route them onto the agent's todo board. `filter`
    /// is provider-agnostic — implementations read only the fields that
    /// apply to their toolkit and translate them into their own action
    /// slug + arguments, then map the upstream payload back into
    /// `NormalizedTask`. Implementations must honour
    /// [`TaskFetchFilter::effective_max`] as an upper bound on the
    /// number of tasks returned.
    ///
    /// Default impl: `Err` — providers without a task surface (e.g.
    /// gmail, slack) opt out, exactly as
    /// [`Self::sync_interval_secs`] returning `None` opts out of the
    /// periodic scheduler.
    async fn fetch_tasks(
        &self,
        ctx: &ProviderContext,
        filter: &TaskFetchFilter,
    ) -> Result<Vec<NormalizedTask>, String> {
        let _ = (ctx, filter);
        Err(format!(
            "[composio:{}] provider has no task-fetch surface",
            self.toolkit_slug()
        ))
    }

    /// List the selectable containers the connected account exposes —
    /// today Notion databases — so the task-source UI can offer a picker
    /// instead of a raw-id text field.
    ///
    /// Default impl: `Err` — providers without a container surface opt out,
    /// mirroring [`Self::fetch_tasks`].
    async fn list_databases(&self, ctx: &ProviderContext) -> Result<Vec<TaskContainer>, String> {
        let _ = ctx;
        Err(format!(
            "[composio:{}] provider has no database/container surface",
            self.toolkit_slug()
        ))
    }

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

                // Mirror the same identity fragment into PROFILE.md so
                // it lands in the agent's prompt context on the next
                // turn (the facets table feeds queries; PROFILE.md
                // feeds the system prompt).
                if let Err(e) = super::profile_md::merge_provider_into_profile_md(
                    &ctx.config.workspace_dir,
                    &profile,
                ) {
                    tracing::warn!(
                        toolkit = %toolkit,
                        error = %e,
                        "[composio:provider] PROFILE.md merge failed (non-fatal)"
                    );
                }
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

/// Build the env var name read by [`resolve_sync_interval_secs`] for a
/// given toolkit slug. Exposed so tests (and `.env.example`) can stay in
/// lockstep with the runtime lookup without re-implementing the casing.
pub fn sync_interval_env_var(toolkit: &str) -> String {
    format!(
        "OPENHUMAN_COMPOSIO_{}_SYNC_INTERVAL_SECS",
        toolkit.to_ascii_uppercase()
    )
}

/// Resolve the effective periodic sync interval (seconds) for a provider.
/// Reads `OPENHUMAN_COMPOSIO_<TOOLKIT>_SYNC_INTERVAL_SECS` if set;
/// otherwise returns `default_secs`. A non-positive or unparseable value
/// is rejected with a `warn` and the default is used — `0` would burn the
/// scheduler in a tight loop, so it is never honoured.
///
/// Each provider's `sync_interval_secs()` impl calls this with its own
/// compile-time default so operators can independently slow down a
/// chatty toolkit (e.g. Slack) without rebuilding.
pub fn resolve_sync_interval_secs(toolkit: &str, default_secs: u64) -> u64 {
    let key = sync_interval_env_var(toolkit);
    match std::env::var(&key) {
        Ok(s) => match s.trim().parse::<u64>() {
            Ok(n) if n >= 1 => n,
            _ => {
                static WARNED: std::sync::Once = std::sync::Once::new();
                WARNED.call_once(|| {
                    tracing::warn!(
                        env = %key,
                        value = %s,
                        default = default_secs,
                        "[composio:provider] sync-interval env override not a positive u64; using default"
                    );
                });
                default_secs
            }
        },
        Err(_) => default_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_interval_env_var_uppercases_slug() {
        assert_eq!(
            sync_interval_env_var("slack"),
            "OPENHUMAN_COMPOSIO_SLACK_SYNC_INTERVAL_SECS"
        );
        assert_eq!(
            sync_interval_env_var("GitHub"),
            "OPENHUMAN_COMPOSIO_GITHUB_SYNC_INTERVAL_SECS"
        );
    }

    /// RAII guard for env var save/restore so the test does not leak
    /// state to siblings within the same process.
    struct EnvGuard {
        key: String,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self {
                key: key.to_string(),
                previous,
            }
        }
        fn unset(key: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self {
                key: key.to_string(),
                previous,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(v) => std::env::set_var(&self.key, v),
                None => std::env::remove_var(&self.key),
            }
        }
    }

    // Bundled into a single `#[test]` so cargo's per-test parallelism
    // does not race on the shared env var. Each scenario explicitly
    // drops its guard before the next so the env is in a known state.
    #[test]
    fn resolve_sync_interval_honors_per_toolkit_env() {
        let _lock = crate::openhuman::config::TEST_ENV_LOCK.lock().unwrap();

        let key = sync_interval_env_var("slack");
        let default = 15 * 60;

        // Unset → default.
        let _g = EnvGuard::unset(&key);
        assert_eq!(resolve_sync_interval_secs("slack", default), default);
        drop(_g);

        // Valid override slows the cadence.
        let _g = EnvGuard::set(&key, "3600");
        assert_eq!(resolve_sync_interval_secs("slack", default), 3600);
        drop(_g);

        // Whitespace tolerated.
        let _g = EnvGuard::set(&key, "  1800  ");
        assert_eq!(resolve_sync_interval_secs("slack", default), 1800);
        drop(_g);

        // Zero rejected (would spin the scheduler).
        let _g = EnvGuard::set(&key, "0");
        assert_eq!(resolve_sync_interval_secs("slack", default), default);
        drop(_g);

        // Garbage rejected.
        let _g = EnvGuard::set(&key, "soon");
        assert_eq!(resolve_sync_interval_secs("slack", default), default);
        drop(_g);

        // Per-toolkit scoping: a different toolkit's var does not bleed
        // into slack's lookup.
        let gmail_key = sync_interval_env_var("gmail");
        let _slack_unset = EnvGuard::unset(&key);
        let _gmail_set = EnvGuard::set(&gmail_key, "120");
        assert_eq!(resolve_sync_interval_secs("slack", default), default);
        assert_eq!(resolve_sync_interval_secs("gmail", default), 120);
    }
}
