//! Mode-aware Composio client construction: [`build_composio_client`]
//! (backend-mode constructor), [`ComposioClientKind`] (the tagged
//! backend/direct union), and [`create_composio_client`] (the factory that
//! reads `config.composio.mode`).

use std::sync::Arc;

use super::connections::ComposioClient;

/// Backend-mode [`ComposioClient`] constructor. **Internal to the
/// composio module** — external callers should use
/// [`create_composio_client`] (factory) or
/// [`crate::agent::subagent_host::user_is_signed_in_to_composio`]
/// (probe) instead.
///
/// Direct exposure leaked through several call sites during the early
/// direct-mode rollout (#1710), where the backend-only nature caused
/// direct-mode users to false-negative the "signed in" check (the
/// agent-tool registration gate, slack sync RPC, `tools.composio_execute`
/// controller, and heartbeat calendar collector all silently dropped
/// direct-mode users). Locking down here prevents future regressions —
/// any new probe or execution path is forced through the mode-aware
/// surface.
///
/// Composio is **always enabled** — there are no configuration flags
/// gating it. The backend URL and auth token come from the shared
/// core defaults (`config.api_url` plus the app-session JWT) via
/// [`crate::integrations::build_client`]. The only reason
/// this returns `None` is that the user isn't signed in to the backend
/// (no JWT). Direct-mode availability is orthogonal — see
/// [`create_composio_client`].
pub(crate) fn build_composio_client(config: &crate::config::Config) -> Option<ComposioClient> {
    let inner = crate::integrations::build_client(config)?;
    Some(ComposioClient::new(inner))
}

// ── Direct-mode factory ─────────────────────────────────────────────
//
// Mirrors `tinyinference-embeddings/src/factory.rs` so anyone reading both can
// pattern-match between domains: string-matched mode, explicit error
// on unknown mode, explicit error when `direct` is selected without an
// API key.

use crate::config::schema::{COMPOSIO_MODE_BACKEND, COMPOSIO_MODE_DIRECT};

// Re-declare the mode strings as local consts so they can be used as
// pattern arms in the `match` below. `use` imports of `pub const &str`
// values get treated as fresh variable bindings in pattern position
// (Rust's pattern grammar accepts only path-qualified constants), so
// pulling them in here resolves to the same `&'static str` values
// without the "unreachable pattern" warning chain.
const MODE_BACKEND_PAT: &str = COMPOSIO_MODE_BACKEND;
const MODE_DIRECT_PAT: &str = COMPOSIO_MODE_DIRECT;

/// Tagged variant returned by [`create_composio_client`].
///
/// `Backend` wraps the existing backend-proxied [`ComposioClient`]
/// (calls `api.tinyhumans.ai/agent-integrations/composio/*`).
///
/// `Direct` wraps the existing direct-mode HTTP wrapper from
/// `composio/tools/direct.rs` that calls
/// `https://backend.composio.dev/api/v{2,3}` with `x-api-key`. The
/// direct client does not currently cover every endpoint the
/// backend-proxied path exposes (no per-toolkit allowlist, no
/// HMAC-verified trigger fan-out, no `/agent-integrations/pricing`),
/// so most existing call-sites continue to use `Backend` for now.
/// Direct-mode integration of the full surface (especially trigger
/// webhooks) is a follow-up.
pub enum ComposioClientKind {
    Backend(ComposioClient),
    /// Held inside an `Arc` so the variant stays cheap to clone — this
    /// matches the rest of the tool registry which juggles
    /// `Arc<dyn Tool>` for the same direct-mode tool elsewhere.
    Direct(Arc<crate::tools::ComposioTool>),
}

pub(crate) fn create_direct_composio_tool_for_api_key(
    config: &crate::config::Config,
    api_key: &str,
) -> anyhow::Result<Arc<crate::tools::ComposioTool>> {
    direct_tool(api_key, config.composio.entity_id.as_str(), None)
}

fn direct_tool(
    api_key: &str,
    entity_id: &str,
    base_urls: Option<&crate::config::ComposioDirectBaseUrls>,
) -> anyhow::Result<Arc<crate::tools::ComposioTool>> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        anyhow::bail!("composio direct api key must not be empty");
    }

    // The direct client takes a `SecurityPolicy` for `Tool::execute`
    // gating, but the factory's job is only to materialize a *client*
    // — it does not actually invoke `execute()` itself, so the
    // default policy is sufficient here. Callers that go through
    // the `Tool` surface re-acquire the live policy from their own
    // context.
    let security = Arc::new(crate::security::SecurityPolicy::default());
    if let Some(urls) = base_urls {
        let tool = crate::tools::ComposioTool::new_with_base_urls(
            api_key,
            Some(entity_id),
            security,
            urls.v2.clone(),
            urls.v3.clone(),
        )?;
        return Ok(Arc::new(tool));
    }
    #[cfg(debug_assertions)]
    let tool = match (
        std::env::var("OPENHUMAN_COMPOSIO_DIRECT_BASE_V2").ok(),
        std::env::var("OPENHUMAN_COMPOSIO_DIRECT_BASE_V3").ok(),
    ) {
        (Some(base_v2), Some(base_v3)) => {
            crate::tools::ComposioTool::new_with_base_urls_for_loopback(
                api_key,
                Some(entity_id),
                security,
                base_v2,
                base_v3,
            )
            .map_err(|e| {
                anyhow::anyhow!("invalid debug composio direct loopback base override: {e}")
            })?
        }
        _ => crate::tools::ComposioTool::new(api_key, Some(entity_id), security),
    };
    #[cfg(not(debug_assertions))]
    let tool = crate::tools::ComposioTool::new(api_key, Some(entity_id), security);
    Ok(Arc::new(tool))
}

impl ComposioClientKind {
    /// Returns `"backend"` or `"direct"` — handy for logging and tests.
    pub fn mode(&self) -> &'static str {
        match self {
            ComposioClientKind::Backend(_) => COMPOSIO_MODE_BACKEND,
            ComposioClientKind::Direct(_) => COMPOSIO_MODE_DIRECT,
        }
    }
}

/// Construct a [`ComposioClientKind`] from the root config.
///
/// Supported `config.composio.mode` values:
///
/// - `"backend"` (default) — backend-proxied; identical to
///   [`build_composio_client`]. Returns
///   `Err("no backend session")` when the user is not signed in.
/// - `"direct"` — BYO key against `backend.composio.dev`. Requires a
///   stored Composio API key under the
///   [`crate::security::credentials::COMPOSIO_DIRECT_PROVIDER`]
///   slot **or** an `api_key` value in `config.composio.api_key`. The
///   stored key takes precedence so the encrypted keychain remains the
///   source of truth — `config.toml` is a fallback for power users.
///
/// A host-pinned credential (`config.composio.host_credential`) takes
/// precedence over both the mode and the credential store.
///
/// Any other mode string is rejected with an explicit error so a typo
/// in `config.toml` fails loud instead of silently downgrading.
pub fn create_composio_client(
    config: &crate::config::Config,
) -> anyhow::Result<ComposioClientKind> {
    if let Some(pinned) = config.composio.host_credential.as_ref() {
        let tool = direct_tool(pinned.api_key(), pinned.entity(), pinned.direct_base_urls())?;
        tracing::debug!("[composio-factory] resolved host-pinned direct variant (key redacted)");
        return Ok(ComposioClientKind::Direct(tool));
    }

    let mode = config.composio.mode.trim();
    tracing::debug!(mode = %mode, "[composio-factory] resolving client");

    match mode {
        // Empty string is treated as the default for forward compatibility
        // with hand-edited configs that omit the field — `serde(default)`
        // already gives us "backend" for missing fields, but a literal
        // empty string in TOML would otherwise be rejected.
        "" | MODE_BACKEND_PAT => {
            let client = build_composio_client(config).ok_or_else(|| {
                anyhow::anyhow!(
                    "composio backend mode unavailable: no backend session token. \
                     Sign in first (auth_store_session)."
                )
            })?;
            tracing::debug!("[composio-factory] resolved backend variant");
            Ok(ComposioClientKind::Backend(client))
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

            let tool = create_direct_composio_tool_for_api_key(config, &api_key)?;
            tracing::debug!(
                key_len = api_key.len(),
                "[composio-factory] resolved direct variant (key redacted)"
            );
            Ok(ComposioClientKind::Direct(tool))
        }
        unknown => {
            tracing::warn!(mode = %unknown, "[composio-factory] unknown composio mode");
            Err(anyhow::anyhow!(
                "unknown composio mode: \"{unknown}\". Supported: \"backend\", \"direct\""
            ))
        }
    }
}
