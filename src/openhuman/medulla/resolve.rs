//! Resolve a configured [`MedullaClient`] from ambient config and credentials.
//!
//! # Why there is no `[medulla]` config section
//!
//! There does not need to be one. The Medulla orchestration API and the
//! OpenHuman backend are the **same deployment**, so `api_url` already
//! addresses it and the existing session token already authenticates against
//! it. A separate `[medulla]` section would be a second source of truth for one
//! endpoint and one credential — exactly the drift this migration exists to
//! remove.
//!
//! `OPENHUMAN_MEDULLA_BASE_URL` remains as an override for pointing a
//! development host at a different Medulla deployment than its OpenHuman one.
//! Unset — the normal case — everything resolves through
//! [`effective_backend_api_url`], the same chain every other hosted-backend
//! call uses: `api_url`, then the `BACKEND_URL` keys, then the prod or staging
//! default selected by `OPENHUMAN_APP_ENV`. Resolving from the raw `api_url`
//! instead would leave Medulla as the one backend surface with no default,
//! reporting itself unconfigured on an install where every other call works.

use crate::api::config::effective_backend_api_url;
use crate::openhuman::config::Config;
use crate::openhuman::security::credentials::session_support::get_session_token;

use super::client::MedullaClient;

/// Environment override for the Medulla backend base URL.
pub const MEDULLA_BASE_URL_ENV: &str = "OPENHUMAN_MEDULLA_BASE_URL";

/// Why a Medulla client could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotConfigured {
    /// Neither the env override nor `api_url` yielded a base URL.
    NoBaseUrl,
    /// No session token is available — the user is signed out.
    NoSessionToken,
}

impl NotConfigured {
    /// A message safe to surface to an operator.
    ///
    /// Deliberately free of URLs and tokens: this text reaches logs and the RPC
    /// error channel.
    pub fn message(&self) -> &'static str {
        match self {
            NotConfigured::NoBaseUrl => {
                "no Medulla backend configured; set OPENHUMAN_MEDULLA_BASE_URL or api_url"
            }
            NotConfigured::NoSessionToken => "not signed in; no session token available",
        }
    }

    /// Stable discriminator for the structured RPC error `data.kind`.
    pub fn kind(&self) -> &'static str {
        match self {
            NotConfigured::NoBaseUrl => "MedullaNoBaseUrl",
            NotConfigured::NoSessionToken => "MedullaNoSessionToken",
        }
    }
}

/// The configured base URL, if any.
///
/// Precedence: `OPENHUMAN_MEDULLA_BASE_URL`, then whatever every other
/// hosted-backend call resolves to — [`effective_backend_api_url`], which
/// applies `config.api_url`, the `BACKEND_URL` env/compile-time keys, and
/// finally the environment-aware default (prod or staging by
/// `OPENHUMAN_APP_ENV`).
///
/// Reading `config.api_url` directly, as this used to, made Medulla the one
/// hosted-backend surface with no default: an install that had never written an
/// explicit `api_url` — the normal case, since every other call falls through
/// to the default — reported "no Medulla backend configured" while auth,
/// billing and integrations all worked. Same deployment, so it resolves the
/// same way. It also inherits the local-AI guard for free: a user whose
/// `api_url` points at Ollama gets the hosted backend here rather than a
/// Medulla client aimed at a model runner.
///
/// Empty or whitespace-only values count as unset, so an exported-but-blank env
/// var does not shadow a working config value.
pub fn base_url(config: &Config) -> Option<String> {
    let from_env = std::env::var(MEDULLA_BASE_URL_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty());

    let source_ident = if from_env.is_some() {
        "env_override"
    } else {
        "effective_backend_api_url"
    };
    log::debug!(
        "[medulla] base_url source={} (URL redacted for security)",
        source_ident
    );

    from_env
        .or_else(|| Some(effective_backend_api_url(&config.api_url)))
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
}

/// Build a client from ambient config + credentials.
///
/// Returns [`NotConfigured`] rather than an opaque error so callers can tell
/// "signed out" (an expected user state the host should render as a notice)
/// from "misconfigured".
pub fn client(config: &Config) -> Result<MedullaClient, NotConfigured> {
    let Some(base) = base_url(config) else {
        log::debug!("[medulla] resolve_client outcome=no_base_url");
        return Err(NotConfigured::NoBaseUrl);
    };

    let token = get_session_token(config)
        .ok()
        .flatten()
        .filter(|t| !t.trim().is_empty());
    let Some(token) = token else {
        log::debug!("[medulla] resolve_client outcome=no_session_token");
        return Err(NotConfigured::NoSessionToken);
    };

    log::debug!("[medulla] resolve_client outcome=ok");
    Ok(MedullaClient::new(base, token))
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
