//! Mode-aware Composio route resolution.
//!
//! Every Composio operation runs in the `tinyconnectors` module
//! (`module_client`). What stays on the host is the decision the module
//! deliberately does not make: which route the current config selects and
//! whether the credential for it exists. [`resolve_composio_route`] answers
//! that with the actionable, user-facing messages (a missing direct key, a
//! missing backend session, a typo in `composio.mode`) instead of the opaque
//! "no connector route" the module reports when it is configured without one
//! (#1710).
//!
//! [`ComposioRoute::Direct`] carries the credential (key and validated base
//! URL) the direct reads are made with, including the host-pinned and loopback
//! overrides the module's single configured route cannot express per call.

use std::sync::Arc;

use super::DirectCredential;
use crate::config::schema::{COMPOSIO_MODE_BACKEND, COMPOSIO_MODE_DIRECT};
use crate::core::slots::HookSlot;

// Re-declare the mode strings as local consts so they can be used as
// pattern arms in the `match` below. `use` imports of `pub const &str`
// values get treated as fresh variable bindings in pattern position
// (Rust's pattern grammar accepts only path-qualified constants), so
// pulling them in here resolves to the same `&'static str` values
// without the "unreachable pattern" warning chain.
const MODE_BACKEND_PAT: &str = COMPOSIO_MODE_BACKEND;
const MODE_DIRECT_PAT: &str = COMPOSIO_MODE_DIRECT;
const MODE_DISABLED_PAT: &str = crate::config::schema::COMPOSIO_MODE_DISABLED;

/// The route [`resolve_composio_route`] selected.
///
/// `Backend` is a unit: the backend-proxied route is reached through the
/// connector module, whose configuration (`modules::connectors::module_config`)
/// carries the base URL and bearer. `Direct` wraps the user's own key and its
/// base URL (see the module docs).
pub enum ComposioRoute {
    Backend,
    /// Held inside an `Arc` so the variant stays cheap to clone.
    Direct(Arc<DirectCredential>),
}

impl ComposioRoute {
    /// Returns `"backend"` or `"direct"` — handy for logging and tests.
    pub fn mode(&self) -> &'static str {
        match self {
            ComposioRoute::Backend => COMPOSIO_MODE_BACKEND,
            ComposioRoute::Direct(_) => COMPOSIO_MODE_DIRECT,
        }
    }
}

/// A registered route builder: resolves the [`ComposioRoute`] for one
/// `composio.mode` value (the empty mode counts as `"backend"`).
pub type ComposioRouteBuilder =
    Arc<dyn Fn(&crate::config::Config) -> anyhow::Result<ComposioRoute> + Send + Sync>;

static ROUTE_BUILDERS: HookSlot<ComposioRouteBuilder> =
    HookSlot::new("integrations::composio::route_builders");

/// Registers `builder` as the route for `mode`, replacing the built-in
/// resolution for that mode. Nothing registered => the built-in routes apply.
pub fn register_composio_route_builder<F>(mode: impl Into<String>, builder: F)
where
    F: Fn(&crate::config::Config) -> anyhow::Result<ComposioRoute> + Send + Sync + 'static,
{
    ROUTE_BUILDERS.register(mode, Arc::new(builder));
}

/// Removes the process-wide route builder for `mode`.
pub fn unregister_composio_route_builder(mode: &str) -> bool {
    ROUTE_BUILDERS.unregister(mode)
}

pub(crate) fn create_direct_client_for_api_key(
    api_key: &str,
) -> anyhow::Result<Arc<DirectCredential>> {
    direct_client(api_key, None)
}

fn direct_client(
    api_key: &str,
    base_urls: Option<&crate::config::ComposioDirectBaseUrls>,
) -> anyhow::Result<Arc<DirectCredential>> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        anyhow::bail!("composio direct api key must not be empty");
    }

    if let Some(urls) = base_urls {
        let client =
            DirectCredential::new_with_base_urls(api_key, urls.v2.clone(), urls.v3.clone())?;
        return Ok(Arc::new(client));
    }
    #[cfg(debug_assertions)]
    let client = match (
        std::env::var("OPENHUMAN_COMPOSIO_DIRECT_BASE_V2").ok(),
        std::env::var("OPENHUMAN_COMPOSIO_DIRECT_BASE_V3").ok(),
    ) {
        (Some(base_v2), Some(base_v3)) => {
            DirectCredential::new_with_base_urls_for_loopback(api_key, base_v2, base_v3).map_err(
                |e| anyhow::anyhow!("invalid debug composio direct loopback base override: {e}"),
            )?
        }
        _ => DirectCredential::new(api_key),
    };
    #[cfg(not(debug_assertions))]
    let client = DirectCredential::new(api_key);
    Ok(Arc::new(client))
}

/// Resolve the [`ComposioRoute`] the root config selects.
///
/// Supported `config.composio.mode` values:
///
/// - `"backend"` (default) — backend-proxied through the connector module.
///   Resolved by the route builder a host registered for it; `Err` when none is
///   registered (the `BACKEND_UNAVAILABLE:` sentinel) or when the user is not
///   signed in.
/// - `"direct"` — BYO key against `backend.composio.dev`. Requires a
///   stored Composio API key under the
///   [`crate::security::credentials::COMPOSIO_DIRECT_PROVIDER`]
///   slot **or** an `api_key` value in `config.composio.api_key`. The
///   stored key takes precedence so the encrypted keychain remains the
///   source of truth — `config.toml` is a fallback for power users.
///
/// - `"disabled"` — Composio is off; always `Err`, so no tools register.
///
/// A host-pinned credential (`config.composio.host_credential`) takes
/// precedence over both the mode and the credential store.
///
/// Any other mode string is rejected with an explicit error so a typo
/// in `config.toml` fails loud instead of silently downgrading.
pub fn resolve_composio_route(config: &crate::config::Config) -> anyhow::Result<ComposioRoute> {
    if let Some(pinned) = config.composio.host_credential.as_ref() {
        let client = direct_client(pinned.api_key(), pinned.direct_base_urls())?;
        tracing::debug!("[composio-factory] resolved host-pinned direct variant (key redacted)");
        return Ok(ComposioRoute::Direct(client));
    }

    let mode = config.composio.mode.trim();
    tracing::debug!(mode = %mode, "[composio-factory] resolving route");

    let builder_key = if mode.is_empty() {
        MODE_BACKEND_PAT
    } else {
        mode
    };
    if let Some(builder) = ROUTE_BUILDERS.get(builder_key) {
        tracing::debug!(mode = %builder_key, "[composio-factory] using the registered route builder");
        return builder(config);
    }

    match mode {
        // Empty string is treated as the default for forward compatibility
        // with hand-edited configs that omit the field — `serde(default)`
        // already gives us "backend" for missing fields, but a literal
        // empty string in TOML would otherwise be rejected.
        "" | MODE_BACKEND_PAT => {
            // The backend-proxied route is served by a host that registered a
            // route builder for it (`register_composio_route_builder`); a core
            // with none has no such route.
            tracing::debug!("[composio-factory] no backend route builder registered");
            anyhow::bail!(
                "{}composio backend mode is not available in this build",
                crate::core::observability::BACKEND_UNAVAILABLE_PREFIX
            );
        }
        MODE_DIRECT_PAT => {
            // Prefer keychain-stored key; fall back to `config.toml`.
            let stored = crate::security::credentials::get_composio_api_key(config)
                .map_err(|e| anyhow::anyhow!("failed to read stored composio api key: {e}"))?;
            let api_key = stored
                .or_else(|| {
                    config
                        .composio
                        .api_key
                        .as_ref()
                        .map(|k| k.trim().to_string())
                        .filter(|k| !k.is_empty())
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "composio direct mode selected but no api key is configured \
                         (set via composio.set_api_key RPC or config.composio.api_key)"
                    )
                })?;

            let client = create_direct_client_for_api_key(&api_key)?;
            tracing::debug!(
                key_len = api_key.len(),
                "[composio-factory] resolved direct variant (key redacted)"
            );
            Ok(ComposioRoute::Direct(client))
        }
        MODE_DISABLED_PAT => {
            tracing::debug!("[composio-factory] composio disabled by config");
            Err(anyhow::anyhow!(
                "composio is disabled (composio.mode = \"disabled\")"
            ))
        }
        unknown => {
            tracing::warn!(mode = %unknown, "[composio-factory] unknown composio mode");
            Err(anyhow::anyhow!(
                "unknown composio mode: \"{unknown}\". Supported: \"backend\", \"direct\", \"disabled\""
            ))
        }
    }
}

#[cfg(test)]
#[path = "factory_tests.rs"]
mod tests;
