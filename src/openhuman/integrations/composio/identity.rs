//! Composio connection identity resolution.
//!
//! Single source of truth for "what is the username on this Composio
//! connection?". Used by the skill preflight gate (`[github]
//! identity_match = "strict"`) and by any future caller that needs to
//! compare the connected account against another subsystem (e.g. local
//! `git config user.name`).
//!
//! The lookup goes through the `tinyconnectors` module's `GetUserProfile`
//! member, which already knows the right Composio action slug for each
//! toolkit (`GITHUB_GET_THE_AUTHENTICATED_USER`, `GMAIL_GET_PROFILE`, …) and
//! the JSON field that holds the username. This used to go through the
//! engine's per-toolkit `ComposioProvider::fetch_user_profile`, deleted by
//! tinymemory v1.13.4 along with the rest of the in-process pipeline —
//! `GetUserProfile` is the module-hosted equivalent, already used by
//! `integrations::composio::ops::providers_ops::composio_get_user_profile`.
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

use crate::openhuman::config::Config;

use super::module_client::{self as connectors, methods};
use super::ops::fetch_connected_integrations;
use super::types::ComposioUserProfileRequest;

/// Resolve the connected account's username for the given Composio
/// toolkit, going through the `tinyconnectors` module's `GetUserProfile`.
///
/// Returns `Some(username)` when:
///   1. The toolkit is currently connected (per
///      [`fetch_connected_integrations`]); AND
///   2. The module's `GetUserProfile` call succeeds AND yields a
///      non-empty `username`.
///
/// Returns `None` for any other case, including a toolkit the module has no
/// identity provider for — the same "no provider registered" outcome the
/// deleted engine's `get_provider(toolkit).is_none()` used to answer, now
/// discovered from the module's own reply instead of a host-side registry
/// lookup. See module docs for the failure contract.
pub async fn connection_identity(config: &Config, toolkit: &str) -> Option<String> {
    let toolkit_norm = toolkit.trim().to_ascii_lowercase();
    if toolkit_norm.is_empty() {
        tracing::debug!("[composio:identity] connection_identity: empty toolkit slug");
        return None;
    }

    // Toolkit must be in the active integrations set. This is the same
    // source of truth Connections uses.
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

    let profile = connectors::call::<_, super::types::ComposioUserProfile>(
        config,
        methods::GET_USER_PROFILE,
        ComposioUserProfileRequest {
            toolkit: toolkit_norm.clone(),
            connection_id: None,
        },
    )
    .await;

    match profile {
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
                "[composio:identity] GetUserProfile failed"
            );
            None
        }
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
