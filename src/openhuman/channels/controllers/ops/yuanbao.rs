//! Yuanbao-specific credential helpers.

use serde_json::Value;

use crate::openhuman::channels::providers::yuanbao::sign::SignManager;
use crate::openhuman::channels::providers::yuanbao::YuanbaoConfig;

/// Read a required non-empty Yuanbao credential field from the connect-channel
/// payload. Returns the trimmed value or an error naming the missing field.
pub(super) fn require_yuanbao_field(
    creds_map: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, String> {
    creds_map
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("missing required {key}"))
}

/// Build the **effective** Yuanbao config that will be used for both
/// preflight verification and persistence.
///
/// Starts from the existing TOML (so manually-installed deployments keep
/// any custom routes), overlays the client-supplied endpoint overrides
/// (`env` / `api_domain` / `ws_domain` / `route_env`), then calls
/// `apply_env_defaults` so the verifier hits the correct cluster — e.g. a
/// user submitting `env = "pre"` is verified against the pre-release
/// sign-token endpoint instead of the default prod one.
///
/// `app_secret` is intentionally left empty: the runtime loads it from
/// the encrypted credentials store at startup, never from `config.toml`.
pub(super) fn build_effective_yuanbao_config(
    base: YuanbaoConfig,
    creds_map: &serde_json::Map<String, Value>,
    app_key: String,
) -> YuanbaoConfig {
    let opt_string = |key: &str| -> Option<String> {
        creds_map
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };

    let mut cfg = base;
    cfg.app_key = app_key;
    cfg.app_secret = String::new();
    if let Some(env) = opt_string("env") {
        cfg.env = env;
    }
    if let Some(api_domain) = opt_string("api_domain") {
        cfg.api_domain = api_domain;
    }
    if let Some(ws_domain) = opt_string("ws_domain") {
        cfg.ws_domain = ws_domain;
    }
    if let Some(route_env) = opt_string("route_env") {
        cfg.route_env = route_env;
    }
    cfg.apply_env_defaults();
    cfg
}

/// Verify Yuanbao credentials against the `sign-token` endpoint before any
/// persistence so invalid `app_key` / `app_secret` surface the upstream API
/// error to the user instead of silently succeeding.
///
/// Takes the **effective** `YuanbaoConfig` already built from the client's
/// overrides + TOML defaults, so the verifier targets whatever cluster the
/// runtime will use after restart.
pub(super) async fn verify_yuanbao_credentials(
    yb_cfg: &YuanbaoConfig,
    app_secret: &str,
) -> Result<(), String> {
    SignManager::new(reqwest::Client::new())
        .get_token(
            &yb_cfg.app_key,
            app_secret,
            &yb_cfg.api_domain,
            &yb_cfg.route_env,
        )
        .await
        .map_err(|e| format!("yuanbao credential verification failed: {e}"))?;
    Ok(())
}
