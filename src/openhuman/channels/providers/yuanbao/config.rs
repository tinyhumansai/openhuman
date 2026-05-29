//! Yuanbao channel configuration.
//!
//! Loaded from `ChannelsConfig.yuanbao` (TOML) and validated before the
//! channel is started. Mirrors the Python `YuanbaoAdapter` configuration
//! surface (hermes-agent `gateway/platforms/yuanbao.py`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::errors::YuanbaoError;

/// Default value for `DeviceInfo.app_version` (server-side `plugin_version`).
///
/// In `yuanbao-openclaw-plugin`'s wire mapping, `plugin_version` represents
/// the *plugin-layer* (npm package) version while `bot_version` represents
/// the *framework runtime* version. OpenHuman has no separate plugin layer;
/// we reuse this slot for the **yuanbao channel provider's** own iteration
/// version. Bump it when the provider's protocol adapter changes in a way
/// the server might care about. The framework runtime version is reported
/// separately via `CARGO_PKG_VERSION` in `DeviceInfo.bot_version`.
pub(crate) const DEFAULT_PLUGIN_VERSION: &str = "0.1.0";

/// Strip legacy `openhuman/` prefix from version strings in config/TOML.
pub(crate) fn strip_version_prefix(version: &str) -> &str {
    version.strip_prefix("openhuman/").unwrap_or(version)
}

/// Production environment endpoints (default).
const PROD_API_DOMAIN: &str = "https://bot.yuanbao.tencent.com";
const PROD_WS_URL: &str = "wss://bot-wss.yuanbao.tencent.com/wss/connection";
/// Pre-release environment endpoints. Opt in via `env = "pre"` in TOML.
const PRE_API_DOMAIN: &str = "https://bot-pre.yuanbao.tencent.com";
const PRE_WS_URL: &str = "wss://bot-wss-pre.yuanbao.tencent.com/wss/connection";

/// User-facing config for the Yuanbao channel.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct YuanbaoConfig {
    /// Application key (`X-ID` header / AuthBind biz_id).
    pub app_key: String,
    /// Application secret — used by the token-sign endpoint.
    pub app_secret: String,
    /// Bot account ID (uid for AuthBind). Optional — when empty, derived
    /// from the AuthBindRsp payload after the first handshake.
    #[serde(default)]
    pub bot_id: String,
    /// Environment selector for endpoint defaults: `"prod"` (default) or `"pre"`.
    /// Only consulted when `api_domain` / `ws_domain` are empty.
    #[serde(default = "default_env")]
    pub env: String,
    /// API base URL. Empty by default — derived from `env` at channel start.
    /// Set explicitly in TOML to point at a custom deployment.
    #[serde(default)]
    pub api_domain: String,
    /// WebSocket base URL. Empty by default — derived from `env` at channel
    /// start. Set explicitly in TOML to point at a custom deployment.
    #[serde(default)]
    pub ws_domain: String,
    /// Optional `route_env` header (canary routing).
    #[serde(default)]
    pub route_env: String,
    /// Optional pre-provisioned token. When empty, the channel calls
    /// `api_domain/api/token/sign` with `(app_key, app_secret)` to fetch one.
    #[serde(default)]
    pub token: String,
    /// Yuanbao channel-provider iteration version, reported in
    /// `AuthBindReq.DeviceInfo.app_version` (server-side `plugin_version`).
    ///
    /// **NOTE:** Despite the legacy `bot_version` TOML key name, this value
    /// fills the *plugin*-version slot on the wire — see
    /// [`DEFAULT_PLUGIN_VERSION`] for the semantic mapping. The framework
    /// runtime version (`DeviceInfo.bot_version` / server `bot_version`)
    /// is sourced from `CARGO_PKG_VERSION` and is **not** user-configurable;
    /// operators who previously set this key intending to override the
    /// framework version should drop it. New configs may use the clearer
    /// `plugin_version` alias.
    #[serde(default = "default_bot_version", alias = "plugin_version")]
    pub bot_version: String,
    /// Optional bot display name — used by the `@bot` mention guard.
    #[serde(default)]
    pub bot_name: String,

    /// DM access policy: `open` / `allowlist` / `closed`.
    #[serde(default = "default_dm_policy")]
    pub dm_access: String,
    /// Group access policy: `open` / `allowlist` / `closed`.
    #[serde(default = "default_group_policy")]
    pub group_access: String,
    /// When `dm_access = "allowlist"`, only these UIDs may DM the bot.
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// When `group_access = "allowlist"`, only these group codes are allowed.
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    /// Owner UID — receives elevated `/admin` commands.
    #[serde(default)]
    pub owner_id: String,

    /// Group messages must `@bot` to be processed (recommended).
    #[serde(default = "default_true")]
    pub group_at_required: bool,

    /// Maximum WS heartbeat interval override (seconds). 0 = use server-driven default.
    #[serde(default)]
    pub heartbeat_interval_secs: u64,
    /// Reconnect retry budget — 0 means use the default cap (100).
    #[serde(default)]
    pub max_reconnect_attempts: u32,

    /// Per-message body length cap before splitting (UTF-8 bytes).
    #[serde(default = "default_max_msg_len")]
    pub max_message_length: usize,
    /// Maximum inbound media file size in MiB.
    #[serde(default = "default_max_media_mb")]
    pub max_media_mb: u32,
}

impl Default for YuanbaoConfig {
    fn default() -> Self {
        Self {
            app_key: String::new(),
            app_secret: String::new(),
            bot_id: String::new(),
            env: default_env(),
            api_domain: String::new(),
            ws_domain: String::new(),
            route_env: String::new(),
            token: String::new(),
            bot_version: default_bot_version(),
            bot_name: String::new(),
            dm_access: default_dm_policy(),
            group_access: default_group_policy(),
            allowed_users: Vec::new(),
            allowed_groups: Vec::new(),
            owner_id: String::new(),
            group_at_required: true,
            heartbeat_interval_secs: 0,
            max_reconnect_attempts: 0,
            max_message_length: default_max_msg_len(),
            max_media_mb: default_max_media_mb(),
        }
    }
}

impl YuanbaoConfig {
    /// Fill empty `api_domain` / `ws_domain` from the configured `env`. The
    /// UI only collects `app_key` + `app_secret`; endpoints are derived
    /// here so the renderer never needs to know about them. TOML values
    /// take precedence (when non-empty), so existing deployments and
    /// custom routes keep working.
    pub fn apply_env_defaults(&mut self) {
        let env = self.env.as_str();
        if self.api_domain.is_empty() {
            self.api_domain = match env {
                "pre" => PRE_API_DOMAIN.into(),
                _ => PROD_API_DOMAIN.into(),
            };
        }
        if self.ws_domain.is_empty() {
            self.ws_domain = match env {
                "pre" => PRE_WS_URL.into(),
                _ => PROD_WS_URL.into(),
            };
        }
    }

    /// Validate required fields. Called at channel construction time so
    /// misconfiguration surfaces early in `start_channels`, not after a
    /// failed WebSocket handshake.
    pub fn validate(&self) -> Result<(), YuanbaoError> {
        if self.app_key.is_empty() {
            return Err(YuanbaoError::Config("`app_key` is required".into()));
        }
        if self.ws_domain.is_empty() {
            return Err(YuanbaoError::Config("`ws_domain` is required".into()));
        }
        if self.token.is_empty() && self.app_secret.is_empty() {
            return Err(YuanbaoError::Config(
                "either `token` or `app_secret` must be set".into(),
            ));
        }
        if self.api_domain.is_empty() && self.token.is_empty() {
            return Err(YuanbaoError::Config(
                "`api_domain` is required when `token` is not pre-provisioned".into(),
            ));
        }
        Ok(())
    }
}

/// Default value for [`YuanbaoConfig::bot_version`] — populates
/// `DeviceInfo.app_version` (server `plugin_version`) when the user config
/// omits the field.
fn default_bot_version() -> String {
    DEFAULT_PLUGIN_VERSION.into()
}

fn default_env() -> String {
    "prod".into()
}

fn default_dm_policy() -> String {
    "open".into()
}

fn default_group_policy() -> String {
    "allowlist".into()
}

fn default_true() -> bool {
    true
}

fn default_max_msg_len() -> usize {
    4500
}

fn default_max_media_mb() -> u32 {
    50
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_invalid() {
        let cfg = YuanbaoConfig::default();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_requires_app_key() {
        let mut cfg = YuanbaoConfig::default();
        cfg.ws_domain = "wss://example".into();
        cfg.token = "tok".into();
        assert!(cfg.validate().is_err());
        cfg.app_key = "ak".into();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_requires_token_or_secret() {
        let mut cfg = YuanbaoConfig::default();
        cfg.app_key = "ak".into();
        cfg.ws_domain = "wss://example".into();
        cfg.api_domain = "https://api".into();
        assert!(cfg.validate().is_err());
        cfg.app_secret = "secret".into();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn apply_env_defaults_fills_prod_when_empty() {
        let mut cfg = YuanbaoConfig::default();
        assert_eq!(cfg.env, "prod");
        cfg.apply_env_defaults();
        assert_eq!(cfg.api_domain, PROD_API_DOMAIN);
        assert_eq!(cfg.ws_domain, PROD_WS_URL);
    }

    #[test]
    fn apply_env_defaults_respects_pre_env() {
        let mut cfg = YuanbaoConfig::default();
        cfg.env = "pre".into();
        cfg.apply_env_defaults();
        assert_eq!(cfg.api_domain, PRE_API_DOMAIN);
        assert_eq!(cfg.ws_domain, PRE_WS_URL);
    }

    #[test]
    fn apply_env_defaults_preserves_explicit_overrides() {
        let mut cfg = YuanbaoConfig::default();
        cfg.api_domain = "https://custom.example".into();
        cfg.ws_domain = "wss://custom.example".into();
        cfg.apply_env_defaults();
        assert_eq!(cfg.api_domain, "https://custom.example");
        assert_eq!(cfg.ws_domain, "wss://custom.example");
    }
}
