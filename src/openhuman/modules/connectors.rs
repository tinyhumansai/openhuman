//! Reaching the `tinyconnectors` module.
//!
//! The connector implementation — the Composio client, the OAuth handoff, the
//! execute pipeline, the trigger archive, the sync providers — lives in the
//! `tinyconnectors` module, not in this crate. This is how the host calls it.
//!
//! # What stays here
//!
//! Three things deliberately did not move, and callers must keep applying them
//! around these calls:
//!
//! - **Egress policy.** [`crate::openhuman::security::egress`] refuses outbound
//!   tool calls under local-only mode and discloses every external transfer.
//!   That is host policy about the *user's* data, and the module cannot see the
//!   reasons behind it. Apply it before calling [`methods::EXECUTE`].
//! - **Which route to use.** Whether the user is signed in, whether they
//!   supplied their own Composio key, and which the product prefers are all
//!   decisions this crate makes. They become the module's configuration blob,
//!   and the module honours it rather than choosing.
//! - **Webhook delivery.** The backend HMAC-verifies Composio webhooks and fans
//!   them out over the user's sockets. The module has no socket, so the
//!   existing trigger subscriber keeps its job.
//!
//! # What the module now owns
//!
//! Scope enforcement. `ListTools` hides what the user's preference forbids and
//! `Execute` refuses it. Do **not** re-filter here against a separately stored
//! preference: two sources of truth for a permission is how one of them ends up
//! stale and permissive.

use std::sync::Mutex;

use serde::de::DeserializeOwned;
use serde::Serialize;
use tinybus::Proxy;
use tinyconnectors_bus::names;

use super::{ops, registry};
use crate::openhuman::config::schema::{COMPOSIO_MODE_BACKEND, COMPOSIO_MODE_DIRECT};
use crate::openhuman::config::Config;

/// The module's id in [`registry`].
pub const MODULE_ID: &str = "tinyconnectors";

/// The names of the members this host calls.
///
/// Re-exported so a call site spells a member through the contract rather than
/// as a string literal: a renamed member is then a compile error here instead
/// of an "unknown method" at runtime on a user's machine.
pub use names::methods;

/// The configuration blob the module is loaded with.
///
/// This is where "which route" is decided, and it is the whole of the host's
/// say in the matter: the module implements both routes and chooses neither.
/// Everything it needs to reach Composio arrives here, and it reads a
/// credential from nowhere else.
///
/// # Errors
///
/// Returns a message when the configured mode cannot be honoured — direct mode
/// with no key, or a mode string that is neither. A typo in `config.toml` fails
/// loudly rather than silently downgrading to the other route, which would send
/// a user's requests somewhere they did not choose.
pub fn module_config(config: &Config) -> Result<serde_json::Value, String> {
    let state_dir = config.workspace_dir.join("state");

    match config.composio.mode.trim() {
        // Empty is the default, for hand-edited configs that omit the field.
        "" | COMPOSIO_MODE_BACKEND => {
            let client = crate::openhuman::integrations::build_client(config).ok_or_else(|| {
                "composio backend mode is unavailable: no backend session token. Sign in first."
                    .to_string()
            })?;
            Ok(serde_json::json!({
                "route": "proxy",
                "base_url": client.backend_url,
                "auth_token": client.auth_token,
                "state_dir": state_dir,
            }))
        }
        COMPOSIO_MODE_DIRECT => {
            // The keychain wins over `config.toml`: the encrypted store is the
            // source of truth, and the file is a fallback for power users.
            let stored = crate::openhuman::security::credentials::get_composio_api_key(config)
                .map_err(|error| format!("failed to read the stored composio api key: {error}"))?;
            let api_key = stored
                .or_else(|| {
                    config
                        .composio
                        .api_key
                        .as_ref()
                        .map(|key| key.trim().to_string())
                        .filter(|key| !key.is_empty())
                })
                .ok_or_else(|| {
                    "composio direct mode is selected but no api key is configured (set it via \
                     composio.set_api_key or config.composio.api_key)"
                        .to_string()
                })?;

            // The module accepts an optional base override and defaults to the
            // real Composio API without one. Before the extraction the host's
            // direct scan honoured OPENHUMAN_COMPOSIO_DIRECT_BASE_V3 (and V2)
            // — test rigs and proxies point Composio at a loopback mock with
            // it — and omitting the field here silently killed that contract:
            // the module dialled backend.composio.dev regardless. Forward it.
            // The module's transport still refuses any non-HTTPS, non-loopback
            // base, so this cannot redirect a real credential to plain HTTP.
            let direct_base = std::env::var("OPENHUMAN_COMPOSIO_DIRECT_BASE_V3")
                .or_else(|_| std::env::var("OPENHUMAN_COMPOSIO_DIRECT_BASE_V2"))
                .ok()
                .map(|base| base.trim().to_string())
                .filter(|base| !base.is_empty());

            let mut payload = serde_json::json!({
                "route": "direct",
                "api_key": api_key,
                "entity_id": config.composio.entity_id,
                "base_url": std::env::var("OPENHUMAN_COMPOSIO_DIRECT_BASE_V3").ok(),
                "state_dir": state_dir,
            });
            if let Some(base) = direct_base {
                payload["base_url"] = serde_json::Value::String(base);
            }
            Ok(payload)
        }
        unknown => Err(format!(
            "unknown composio mode: \"{unknown}\". Supported: \"{COMPOSIO_MODE_BACKEND}\", \
             \"{COMPOSIO_MODE_DIRECT}\""
        )),
    }
}

/// The route description last sent to the module, as a fingerprint.
///
/// Only a hash is kept: comparing routes means comparing bearer tokens, and a
/// process-lifetime static holding one in cleartext is a credential sitting
/// somewhere nothing needs it.
fn last_route() -> &'static Mutex<Option<u64>> {
    static LAST: std::sync::OnceLock<Mutex<Option<u64>>> = std::sync::OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

/// Hash a route description so it can be compared without being stored.
fn fingerprint(route: &serde_json::Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    route.to_string().hash(&mut hasher);
    hasher.finish()
}

/// Tell the module which route to use, if that answer has changed.
///
/// The module is registered `Lazy`, so it is commonly loaded the first time
/// anything touches a connector — which for most sessions is *before* the user
/// signs in. Its load-time configuration would then be routeless, and a module
/// that only took a route at load would leave that user unable to reach
/// Composio until they restarted the application. Sign-out is the same problem
/// reversed: the module would keep a bearer that now answers 401 to everything.
///
/// So the route is reconciled on every call rather than only at load. In steady
/// state that is a hash and a comparison; a bus round-trip happens only when the
/// answer actually changed.
///
/// A config that cannot name a route — signed out, direct mode with no key —
/// sends `{"route": "none"}` rather than nothing. Leaving the old route in
/// place would have the module answering 401 to everything with a bearer the
/// user's session no longer owns, and "your account is broken" is a bad way to
/// tell someone they are signed out.
async fn ensure_routed(config: &Config, proxy: &Proxy) -> Result<(), String> {
    let mut route =
        module_config(config).unwrap_or_else(|_| serde_json::json!({ "route": "none" }));
    // `state_dir` is load-time only: the trigger archive opens once, and moving
    // it later would strand the history already written there.
    if let Some(object) = route.as_object_mut() {
        object.remove("state_dir");
    }

    let current = fingerprint(&route);
    if *last_route()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        == Some(current)
    {
        return Ok(());
    }

    proxy
        .call::<serde_json::Value>(methods::CONFIGURE, (route,))
        .await
        .map_err(|error| format!("{}: {error}", methods::CONFIGURE))?;

    // Recorded only after the module accepted it, so a failed reconfiguration
    // is retried on the next call rather than remembered as done.
    *last_route()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(current);
    Ok(())
}

/// A proxy to the connector module, loading it if this is the first call.
///
/// The module is registered `Lazy`, so this is where a user with connected
/// accounts pays to load it and a user without one never does.
///
/// # Errors
///
/// Returns a message naming what went wrong: modules disabled in configuration,
/// the artifact missing or failing its digest check, or the bus refusing the
/// proxy.
pub async fn proxy(config: &Config) -> Result<Proxy, String> {
    ops::ensure_loaded(config, MODULE_ID).await?;

    let record =
        registry::find(MODULE_ID).ok_or_else(|| format!("unknown module '{MODULE_ID}'"))?;
    let runtime = super::host::runtime()
        .await
        .map_err(|error| format!("the module runtime is unavailable: {error}"))?;

    let proxy = runtime
        .proxy(record.bus_name, record.object_path)
        .map_err(|error| format!("could not reach '{MODULE_ID}': {error}"))?;

    ensure_routed(config, &proxy).await?;
    Ok(proxy)
}

/// Call one member with an argument and decode its reply.
///
/// # Errors
///
/// Returns the member's failure as the module rendered it.
///
/// Note what is *not* an error: a Composio action the provider refused comes
/// back as a successful reply carrying `successful: false` and a formatted
/// message. A caller that checks only for `Err` here will report a failed send
/// as a success.
pub async fn call<Request, Reply>(
    config: &Config,
    member: &str,
    request: Request,
) -> Result<Reply, String>
where
    Request: Serialize + Send,
    Reply: DeserializeOwned,
{
    let proxy = proxy(config).await?;
    proxy
        .call::<Reply>(member, (request,))
        .await
        .map_err(|error| format!("{member}: {error}"))
}

/// Call a long-running member with a deadline sized for it.
///
/// The default bus deadline (30s) fits request-shaped members. `Sync` is not
/// one: the module pages a whole connected account through inside the call —
/// "a full sync is minutes of paging" is its own documentation — and a 30s
/// deadline made the host report failure while the module went on to finish
/// (observed live: timeout at 30s, `run finished … ingested=200` at 38s, and
/// a Sync button that spun forever on a run that had actually succeeded).
///
/// # Errors
///
/// As [`call`].
pub async fn call_slow<Request, Reply>(
    config: &Config,
    member: &str,
    request: Request,
) -> Result<Reply, String>
where
    Request: Serialize + Send,
    Reply: DeserializeOwned,
{
    const SLOW_MEMBER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);
    let proxy = proxy(config).await?.with_timeout(SLOW_MEMBER_TIMEOUT);
    proxy
        .call::<Reply>(member, (request,))
        .await
        .map_err(|error| format!("{member}: {error}"))
}

/// Call a member that takes no arguments.
///
/// # Errors
///
/// As [`call`].
pub async fn call_bare<Reply: DeserializeOwned>(
    config: &Config,
    member: &str,
) -> Result<Reply, String> {
    let proxy = proxy(config).await?;
    proxy
        .call::<Reply>(member, ())
        .await
        .map_err(|error| format!("{member}: {error}"))
}

#[cfg(test)]
#[path = "connectors_tests.rs"]
mod tests;
