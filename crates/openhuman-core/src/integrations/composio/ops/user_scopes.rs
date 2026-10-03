//! Per-toolkit agent scope preferences, stored in
//! `<workspace>/integrations/composio_user_scopes.json` through
//! [`crate::integrations::composio::file_store`].
//!
//! The file maps a normalised toolkit slug to its [`UserScopePref`]. The
//! `composio.{get,set}_user_scopes` handlers, the agent's visible tool list
//! and the flows capability gate all read through [`load_or_default`].

use std::collections::BTreeMap;

use crate::config::Config;
use crate::integrations::composio::file_store::{self, USER_SCOPES_FILE};

/// The stored type: the contract's scope pref, which is also the wire shape of
/// both handlers' replies.
pub(crate) use crate::integrations::composio::providers::UserScopePref;

/// The stored map: normalised toolkit slug → pref.
type ScopeFile = BTreeMap<String, UserScopePref>;

/// The row key for a toolkit — trimmed and ASCII-lowercased, so `"GitHub"`,
/// `" github "` and `"github"` reach one row.
fn kv_key(toolkit: &str) -> String {
    toolkit.trim().to_ascii_lowercase()
}

/// The scope pref stored for `toolkit`, or the default when none is.
///
/// **This read fails open**: an unreadable file lands on the same default
/// (`read+write`, no `admin`) a user who never opened the toggle gets. A failed
/// read is not evidence the user revoked anything, and refusing every
/// integration on a transient I/O error would break a working app. The
/// failure is logged so "defaulted because nothing is stored" stays
/// distinguishable from "defaulted because the store is unreadable". The
/// **write** below does not fail open — see [`save`].
pub(crate) async fn load_or_default(config: &Config, toolkit: &str) -> UserScopePref {
    let key = kv_key(toolkit);
    if key.is_empty() {
        return UserScopePref::default();
    }
    let path = file_store::path(config, USER_SCOPES_FILE);
    match file_store::load::<ScopeFile>(&path).await {
        Ok(mut prefs) => match prefs.remove(&key) {
            Some(pref) => {
                tracing::debug!(
                    toolkit = %key,
                    read = pref.read,
                    write = pref.write,
                    admin = pref.admin,
                    "[composio][scopes] pref loaded"
                );
                pref
            }
            None => {
                tracing::debug!(
                    toolkit = %key,
                    "[composio][scopes] no pref stored, using default (read+write)"
                );
                UserScopePref::default()
            }
        },
        Err(error) => {
            tracing::warn!(
                toolkit = %key,
                %error,
                "[composio][scopes] pref store unreadable, falling back to default"
            );
            UserScopePref::default()
        }
    }
}

/// Persist the scope pref for `toolkit`.
///
/// **This one fails closed.** A write that reported success without storing
/// anything would leave the user looking at a toggle they just moved while the
/// agent kept the old permissions. Every failure is an `Err` the RPC surfaces.
///
/// # Errors
///
/// An empty toolkit, or the scope file cannot be read or written.
pub(crate) async fn save(
    config: &Config,
    toolkit: &str,
    pref: UserScopePref,
) -> Result<(), String> {
    let key = kv_key(toolkit);
    if key.is_empty() {
        return Err("user_scopes: toolkit must not be empty".to_string());
    }
    let path = file_store::path(config, USER_SCOPES_FILE);
    let _guard = file_store::lock().await;
    let mut prefs: ScopeFile = file_store::load(&path).await?;
    prefs.insert(key.clone(), pref);
    file_store::save(&path, &prefs).await?;

    tracing::info!(
        toolkit = %key,
        read = pref.read,
        write = pref.write,
        admin = pref.admin,
        "[composio][scopes] pref saved"
    );
    Ok(())
}

#[cfg(test)]
#[path = "user_scopes_tests.rs"]
mod tests;
