//! Composio connection identity resolution.
//!
//! Single source of truth for "what is the username on this Composio
//! connection?". Used by the skill preflight gate (`[github]
//! identity_match = "strict"`) and by any future caller that needs to
//! compare the connected account against another subsystem (e.g. local
//! `git config user.name`).
//!
//! The lookup goes through the per-toolkit
//! [`ComposioProvider::fetch_user_profile`](crate::openhuman::memory_sync::composio::providers::ComposioProvider::fetch_user_profile)
//! call, which already knows the right Composio action slug for each
//! toolkit (`GITHUB_GET_THE_AUTHENTICATED_USER`,
//! `GMAIL_GET_PROFILE`, …) and the JSON field that holds the username.
//!
//! ## Failure surface
//!
//! Everything in this module is best-effort and returns `Option`:
//!
//! - toolkit not registered → `None`
//! - user not signed in / no active connection for the toolkit → `None`
//! - Composio call fails / returns no username → `None`
//!
//! Callers MUST treat `None` as "couldn't resolve" rather than
//! "username is empty". The preflight gate uses this contract to map
//! `None` into a clear "GitHub identity not resolved — reconnect via
//! `composio_authorize github`" error.

use std::sync::Arc;

use crate::openhuman::config::Config;

use super::ops::fetch_connected_integrations;
use super::providers::{get_provider, ProviderContext};

/// Resolve the connected account's username for the given Composio
/// toolkit, going through the existing per-provider `fetch_user_profile`
/// path.
///
/// Returns `Some(username)` when:
///   1. The toolkit has a registered provider; AND
///   2. The toolkit is currently connected (per
///      [`fetch_connected_integrations`]); AND
///   3. The provider's `fetch_user_profile` call succeeds AND yields a
///      non-empty `username`.
///
/// Returns `None` for any other case. See module docs for the failure
/// contract.
pub async fn connection_identity(config: &Config, toolkit: &str) -> Option<String> {
    let toolkit_norm = toolkit.trim().to_ascii_lowercase();
    if toolkit_norm.is_empty() {
        tracing::debug!("[composio:identity] connection_identity: empty toolkit slug");
        return None;
    }

    // (1) Provider must exist for this toolkit.
    let provider = match get_provider(&toolkit_norm) {
        Some(p) => p,
        None => {
            tracing::debug!(
                toolkit = %toolkit_norm,
                "[composio:identity] no provider registered for toolkit"
            );
            return None;
        }
    };

    // (2) Toolkit must be in the active integrations set. This is the
    //     same source of truth Settings → Connections uses.
    let connections = fetch_connected_integrations(config).await;
    let matching = connections
        .iter()
        .find(|c| c.toolkit.eq_ignore_ascii_case(&toolkit_norm));
    if matching.is_none() {
        tracing::debug!(
            toolkit = %toolkit_norm,
            "[composio:identity] toolkit not in active integrations"
        );
        return None;
    }

    // (3) Build a provider context and call fetch_user_profile.
    //     `ProviderContext::from_config` probes the Composio factory and
    //     returns `None` when the user isn't signed in at all — same
    //     short-circuit other consumers rely on.
    let ctx = ProviderContext::from_config(Arc::new(config.clone()), &toolkit_norm, None)?;
    match provider.fetch_user_profile(&ctx).await {
        Ok(profile) => {
            let username = profile.username.as_deref().map(str::trim).unwrap_or("");
            if username.is_empty() {
                tracing::debug!(
                    toolkit = %toolkit_norm,
                    "[composio:identity] provider returned empty username"
                );
                None
            } else {
                tracing::debug!(
                    toolkit = %toolkit_norm,
                    resolved = true,
                    "[composio:identity] resolved username"
                );
                Some(username.to_string())
            }
        }
        Err(e) => {
            tracing::debug!(
                toolkit = %toolkit_norm,
                error = %e,
                "[composio:identity] fetch_user_profile failed"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_sync::composio::providers::{
        register_provider, ComposioProvider, ProviderArc, ProviderUserProfile, SyncOutcome,
        SyncReason,
    };
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Test provider that returns a fixed username (or fails, when
    /// `fail` is set). We don't go through Composio at all — the
    /// preflight gate just needs the provider's `username` field.
    struct StubProvider {
        slug: &'static str,
        username: Option<&'static str>,
        fail: bool,
        calls: AtomicUsize,
    }

    impl StubProvider {
        fn new(slug: &'static str, username: Option<&'static str>) -> Self {
            Self {
                slug,
                username,
                fail: false,
                calls: AtomicUsize::new(0),
            }
        }
        fn failing(slug: &'static str) -> Self {
            Self {
                slug,
                username: None,
                fail: true,
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl ComposioProvider for StubProvider {
        fn toolkit_slug(&self) -> &'static str {
            self.slug
        }

        async fn fetch_user_profile(
            &self,
            _ctx: &ProviderContext,
        ) -> Result<ProviderUserProfile, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err("stub provider: forced failure".to_string());
            }
            Ok(ProviderUserProfile {
                toolkit: self.slug.to_string(),
                username: self.username.map(|s| s.to_string()),
                ..Default::default()
            })
        }

        async fn sync(
            &self,
            _ctx: &ProviderContext,
            reason: SyncReason,
        ) -> Result<SyncOutcome, String> {
            Ok(SyncOutcome {
                toolkit: self.slug.to_string(),
                reason: reason.as_str().to_string(),
                ..Default::default()
            })
        }
    }

    fn fresh_config_in_workspace(tmp: &std::path::Path) -> Config {
        let mut config = Config::default();
        config.config_path = tmp.join("config.toml");
        config.workspace_dir = tmp.join("workspace");
        config.secrets.encrypt = false;
        config
    }

    #[tokio::test]
    async fn empty_toolkit_short_circuits_to_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = fresh_config_in_workspace(tmp.path());
        assert!(connection_identity(&config, "").await.is_none());
        assert!(connection_identity(&config, "   ").await.is_none());
    }

    #[tokio::test]
    async fn unknown_toolkit_returns_none_without_provider_call() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = fresh_config_in_workspace(tmp.path());
        // Toolkit slug that has no registered provider.
        assert!(connection_identity(&config, "not-a-real-toolkit-xyz")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn no_active_connection_short_circuits_before_provider_call() {
        // Register a provider but no connections exist for the toolkit
        // → identity helper should return None without calling
        // fetch_user_profile.
        let stub: ProviderArc = Arc::new(StubProvider::new(
            "stub-no-active",
            Some("would-not-be-returned"),
        ));
        register_provider(stub.clone());

        let tmp = tempfile::tempdir().expect("tempdir");
        let config = fresh_config_in_workspace(tmp.path());
        // Default config has no Composio auth → fetch_connected_integrations
        // returns an empty vec, so the toolkit is not "in active".
        let username = connection_identity(&config, "stub-no-active").await;
        assert!(username.is_none(), "must short-circuit when not connected");
    }
}
