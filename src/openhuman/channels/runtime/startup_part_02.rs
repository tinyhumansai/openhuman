
/// Render the agent's current filesystem-access boundaries as a system-prompt
/// section. Advisory only: the `SecurityPolicy` enforces these regardless of
/// what the model believes, but stating them keeps the model from wasting turns
/// attempting actions the runtime will deny.
fn format_access_context(security: &SecurityPolicy) -> String {
    use crate::openhuman::security::{AutonomyLevel, TrustedAccess};

    let mode = match security.autonomy {
        AutonomyLevel::ReadOnly => "read-only (observe only; no writes or shell commands)",
        AutonomyLevel::Supervised => "supervised (acts; risky operations require approval)",
        AutonomyLevel::Full => "full (autonomous within policy bounds)",
    };
    let mut s =
        String::from("\n\n## Host access (enforced by the runtime — you cannot exceed this)\n");
    s.push_str(&format!("- Access mode: {mode}\n"));
    s.push_str(&format!(
        "- Workspace: {} ({})\n",
        security.workspace_dir.display(),
        if security.workspace_only {
            "file access confined to the workspace"
        } else {
            "workspace_only is OFF"
        }
    ));
    if security.trusted_roots.is_empty() {
        s.push_str("- Trusted roots outside the workspace: none granted\n");
    } else {
        s.push_str("- Trusted roots outside the workspace:\n");
        for root in &security.trusted_roots {
            let access = match root.access {
                TrustedAccess::Read => "read-only",
                TrustedAccess::ReadWrite => "read+write",
            };
            s.push_str(&format!("    - {} ({access})\n", root.path));
        }
    }
    s.push_str(&format!(
        "- OS package installation: {}\n",
        if security.allow_tool_install {
            "allowed via install_tool"
        } else {
            "disabled"
        }
    ));
    s.push_str(
        "Credential stores (~/.ssh, ~/.gnupg, ~/.aws) are always blocked. \
         Use detect_tools to check what's installed before assuming a tool exists.\n",
    );
    s
}

/// Best-effort fill of `yb_cfg.app_secret` from the encrypted credentials
/// store when TOML doesn't already carry one.
///
/// `app_secret` is intentionally not persisted in `config.toml` (see the
/// `yuanbao` branch in `controllers/ops.rs`). Existing TOML values still
/// win so manually-installed deployments don't break. Returns the
/// (possibly-modified) config; logging is the only side effect on failure.
///
/// The stored secret is **only** copied when the stored profile's
/// `app_key` matches `yb_cfg.app_key`. Without that guard, editing
/// `app_key` in `config.toml` would silently pair a fresh key with a
/// stale secret on next startup, and the channel would fail auth until
/// the user reconnected or cleared credentials manually.
fn resolve_yuanbao_app_secret(
    mut yb_cfg: crate::openhuman::channels::providers::yuanbao::YuanbaoConfig,
    config: &Config,
) -> crate::openhuman::channels::providers::yuanbao::YuanbaoConfig {
    if !yb_cfg.app_secret.is_empty() {
        return yb_cfg;
    }
    let auth = crate::openhuman::security::credentials::AuthService::from_config(config);
    match auth.get_profile("channel:yuanbao:api_key", None) {
        Ok(Some(profile)) => {
            let stored_app_key = profile.metadata.get("app_key").map(String::as_str);
            if stored_app_key != Some(yb_cfg.app_key.as_str()) {
                tracing::warn!(
                    "[channels] yuanbao stored credentials are for a different app_key (toml={:?}, store={:?}); reconnect the channel to refresh the secret",
                    yb_cfg.app_key,
                    stored_app_key,
                );
            } else if let Some(secret) = profile.metadata.get("app_secret") {
                yb_cfg.app_secret = secret.clone();
            }
        }
        Ok(None) => {
            tracing::warn!(
                "[channels] yuanbao credentials missing — connect the channel again from the UI"
            );
        }
        Err(e) => {
            tracing::warn!("[channels] failed to load yuanbao credentials: {e}");
        }
    }
    yb_cfg
}

/// Best-effort fill of `email_cfg.password` from the encrypted credentials store
/// when TOML doesn't already carry one.
///
/// The IMAP/SMTP `password` is intentionally not persisted in `config.toml` (see
/// `persist_email_config` in `controllers/ops/connect.rs`); it lives only in the
/// credentials store under `channel:email:api_key`. Existing TOML values still
/// win so manually-installed deployments keep working. The stored secret is only
/// copied when the stored profile's `username` matches, so editing `username` in
/// `config.toml` can't silently pair a fresh account with a stale password.
fn resolve_email_password(
    mut email_cfg: crate::openhuman::channels::email_channel::EmailConfig,
    config: &Config,
) -> crate::openhuman::channels::email_channel::EmailConfig {
    if !email_cfg.password.is_empty() {
        return email_cfg;
    }
    let auth = crate::openhuman::security::credentials::AuthService::from_config(config);
    match auth.get_profile("channel:email:api_key", None) {
        Ok(Some(profile)) => {
            let stored_username = profile.metadata.get("username").map(String::as_str);
            if stored_username != Some(email_cfg.username.as_str()) {
                tracing::warn!(
                    "[channels] email stored credentials are for a different username (toml={:?}, store={:?}); reconnect the channel to refresh the password",
                    email_cfg.username,
                    stored_username,
                );
            } else if let Some(password) = profile.metadata.get("password") {
                email_cfg.password = password.clone();
            }
        }
        Ok(None) => {
            tracing::warn!(
                "[channels] email credentials missing — connect the channel again from the UI"
            );
        }
        Err(e) => {
            tracing::warn!("[channels] failed to load email credentials: {e}");
        }
    }
    email_cfg
}

/// Supplies `tinychannels`' provider factory with this host's HTTP clients.
///
/// The factory is transport-agnostic on purpose: proxy configuration, TLS
/// backend and timeouts are the embedding host's business. This is where
/// OpenHuman's runtime proxy settings get applied, per channel, using the same
/// `channel.<name>` identifiers the config UI shows.
struct RuntimeProxyClients;

impl tinychannels::HttpClientFactory for RuntimeProxyClients {
    fn client_for(&self, channel: &str) -> reqwest::Client {
        crate::openhuman::config::build_runtime_proxy_client(channel)
    }

    /// Signal talks to a local `signal-cli` HTTP bridge that may simply not be
    /// running. Without a connect timeout that presents as a hang at startup
    /// rather than an error, so the default is overridden to keep the 10s bound
    /// this host has always used.
    fn signal_client(&self) -> reqwest::Client {
        crate::openhuman::config::apply_runtime_proxy_to_builder(
            reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(10)),
            "channel.signal",
        )
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
    }
}

/// Resolve channel secrets that live outside the config file.
///
/// `tinychannels::build_channels` cannot do this: secrets may sit in the
/// keyring, an environment variable or the config, and only this host knows
/// which. It therefore expects an already-hydrated config, and this is where
/// that happens — on a clone, so the persisted config is never mutated.
fn hydrate_channel_credentials(config: &Config) -> tinychannels::ChannelsConfig {
    let mut hydrated = config.channels_config.clone();
    if let Some(email_cfg) = hydrated.email.take() {
        hydrated.email = Some(resolve_email_password(email_cfg, config));
    }
    if let Some(yb_cfg) = hydrated.yuanbao.take() {
        hydrated.yuanbao = Some(resolve_yuanbao_app_secret(yb_cfg, config));
    }
    hydrated
}
