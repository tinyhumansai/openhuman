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
mod tests {
    use super::*;

    /// Serialize env mutation via the crate-wide backend env lock:
    /// `base_url` reads process-globals that `api::config`, `core::cli_tests`,
    /// and `medulla::ops` tests also mutate — a module-local lock cannot
    /// prevent that cross-module race.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::api::config::backend_env_test_lock()
    }

    /// RAII guard to snapshot and restore a process-global environment variable.
    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, val: &str) -> Self {
            let prev = std::env::var(key).ok();
            // SAFETY: caller holds ENV_LOCK guard.
            unsafe { std::env::set_var(key, val) };
            Self { key, prev }
        }

        fn remove(key: &'static str) -> Self {
            let prev = std::env::var(key).ok();
            // SAFETY: caller holds ENV_LOCK guard.
            unsafe { std::env::remove_var(key) };
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                // SAFETY: caller's ENV_LOCK guard is still alive during drop.
                Some(v) => unsafe { std::env::set_var(self.key, v) },
                None => unsafe { std::env::remove_var(self.key) },
            }
        }
    }

    fn config_with_api_url(url: Option<&str>) -> Config {
        let mut config = Config::default();
        config.api_url = url.map(str::to_string);
        config
    }

    #[test]
    fn env_override_wins_over_api_url() {
        let _guard = env_lock();
        let _env = EnvGuard::set(MEDULLA_BASE_URL_ENV, "https://medulla.example");
        let resolved = base_url(&config_with_api_url(Some("https://api.example")));
        assert_eq!(resolved.as_deref(), Some("https://medulla.example"));
    }

    #[test]
    fn falls_back_to_api_url_when_env_unset() {
        let _guard = env_lock();
        let _env = EnvGuard::remove(MEDULLA_BASE_URL_ENV);
        let resolved = base_url(&config_with_api_url(Some("https://api.example/")));
        assert_eq!(resolved.as_deref(), Some("https://api.example"));
    }

    #[test]
    fn blank_env_does_not_shadow_api_url() {
        // An exported-but-empty var is a common shell accident; treating it as
        // "configured" would break an otherwise-working setup.
        let _guard = env_lock();
        let _env = EnvGuard::set(MEDULLA_BASE_URL_ENV, "   ");
        let resolved = base_url(&config_with_api_url(Some("https://api.example")));
        assert_eq!(resolved.as_deref(), Some("https://api.example"));
    }

    #[test]
    fn an_unconfigured_install_falls_back_to_the_hosted_backend() {
        // Medulla and the OpenHuman backend are one deployment, so an install
        // that never wrote an explicit `api_url` — the normal case — must reach
        // the same host every other backend call defaults to, not report itself
        // unconfigured while auth and billing work.
        let _guard = env_lock();
        let _env_medulla = EnvGuard::remove(MEDULLA_BASE_URL_ENV);
        let _env_backend = EnvGuard::remove("BACKEND_URL");
        let _env_vite = EnvGuard::remove("VITE_BACKEND_URL");
        assert_eq!(
            base_url(&config_with_api_url(None)),
            Some(crate::api::config::effective_backend_api_url(&None))
        );
    }

    #[test]
    fn a_local_model_runner_url_does_not_become_the_medulla_host() {
        // `api_url` doubles as the inference endpoint. Pointing it at Ollama
        // must not aim the Medulla client at a model runner that 404s every
        // path it speaks — the same guard every other backend call gets.
        let _guard = env_lock();
        let _env_medulla = EnvGuard::remove(MEDULLA_BASE_URL_ENV);
        let _env_backend = EnvGuard::remove("BACKEND_URL");
        let _env_vite = EnvGuard::remove("VITE_BACKEND_URL");
        let resolved = base_url(&config_with_api_url(Some("http://localhost:11434")));
        // Assert that the local model runner URL does not become the Medulla host;
        // instead, it falls back to the effective backend URL (same as all other backend calls).
        assert_eq!(
            resolved,
            Some(crate::api::config::effective_backend_api_url(&None))
        );
    }

    #[test]
    fn trailing_slashes_are_trimmed_so_paths_do_not_double_up() {
        let _guard = env_lock();
        let _env = EnvGuard::set(MEDULLA_BASE_URL_ENV, "https://medulla.example///");
        let resolved = base_url(&config_with_api_url(None));
        assert_eq!(resolved.as_deref(), Some("https://medulla.example"));
    }

    #[test]
    fn not_configured_messages_carry_no_secrets() {
        for reason in [NotConfigured::NoBaseUrl, NotConfigured::NoSessionToken] {
            let msg = reason.message();
            assert!(!msg.contains("http"), "message must not leak a URL: {msg}");
            assert!(!reason.kind().is_empty());
        }
    }
}
