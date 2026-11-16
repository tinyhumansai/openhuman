//! Process-wide runtime flags read from environment variables, and the
//! agent-server status probe.

use serde::Serialize;
use serde_json::json;

use crate::core::Outcome;

pub(crate) fn env_flag_enabled(key: &str) -> bool {
    matches!(
        std::env::var(key).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

/// Returns the core RPC URL from environment variables or a default value.
pub fn core_rpc_url_from_env() -> String {
    std::env::var("OPENHUMAN_CORE_RPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:7788/rpc".to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeFlagsOut {
    pub browser_allow_all: bool,
    pub log_prompts: bool,
}

pub(crate) const BROWSER_ALLOW_ALL_ENV: &str = "OPENHUMAN_BROWSER_ALLOW_ALL";
pub(crate) const BROWSER_ALLOW_ALL_RPC_ENABLE_ENV: &str = "OPENHUMAN_BROWSER_ALLOW_ALL_RPC_ENABLE";

/// Returns the current state of runtime-only flags.
pub fn get_runtime_flags() -> Outcome<RuntimeFlagsOut> {
    Outcome::single_log(runtime_flags(), "runtime flags read")
}

pub(crate) fn runtime_flags() -> RuntimeFlagsOut {
    RuntimeFlagsOut {
        browser_allow_all: env_flag_enabled(BROWSER_ALLOW_ALL_ENV),
        log_prompts: env_flag_enabled("OPENHUMAN_LOG_PROMPTS"),
    }
}

/// Updates the `OPENHUMAN_BROWSER_ALLOW_ALL` environment flag.
///
/// **Security note:** when enabled, this disables the browser tool's
/// per-domain allowlist for the entire process. Both transitions are
/// audit-logged at WARN level with a `[SECURITY]` prefix so operators
/// (and `journalctl -g '\[SECURITY\]'` style scrapes) can spot
/// allowlist toggles in the live log stream.
///
/// `is_private_host` checks still apply to the resolved IP, so this
/// flag does not unlock loopback / RFC1918 destinations.
pub fn set_browser_allow_all(enabled: bool) -> Result<Outcome<RuntimeFlagsOut>, String> {
    if enabled && !env_flag_enabled(BROWSER_ALLOW_ALL_RPC_ENABLE_ENV) {
        tracing::warn!(
            "[SECURITY] refused browser allow-all enable via RPC: \
             set {BROWSER_ALLOW_ALL_ENV}=1 at startup or explicitly set \
             {BROWSER_ALLOW_ALL_RPC_ENABLE_ENV}=1 before using the runtime toggle"
        );
        return Err(format!(
            "Refusing to enable {BROWSER_ALLOW_ALL_ENV} via RPC. Start OpenHuman with \
             {BROWSER_ALLOW_ALL_ENV}=1, or set {BROWSER_ALLOW_ALL_RPC_ENABLE_ENV}=1 for an \
             explicit operator-approved runtime override."
        ));
    }

    let was_enabled = env_flag_enabled(BROWSER_ALLOW_ALL_ENV);
    if enabled {
        unsafe {
            std::env::set_var(BROWSER_ALLOW_ALL_ENV, "1");
        }
    } else {
        unsafe {
            std::env::remove_var(BROWSER_ALLOW_ALL_ENV);
        }
    }
    let flags = runtime_flags();
    let now_enabled = flags.browser_allow_all;

    if was_enabled != now_enabled {
        if now_enabled {
            tracing::warn!(
                "[SECURITY] browser allow-all enabled via RPC: \
                 per-domain allowlist is now bypassed for all sessions \
                 (private-host check still applies)"
            );
        } else {
            tracing::info!(
                "[SECURITY] browser allow-all disabled via RPC: \
                 per-domain allowlist re-enforced"
            );
        }
    }

    let log_msg = if now_enabled {
        "[SECURITY] browser allow-all flag set to enabled"
    } else {
        "[SECURITY] browser allow-all flag set to disabled"
    };
    Ok(Outcome::single_log(flags, log_msg))
}

/// Returns the operational status of the agent server.
pub fn agent_server_status() -> Outcome<serde_json::Value> {
    let running = openhuman_platform::service::mock_agent_running().unwrap_or(true);
    log::info!("[config] agent_server_status requested: running={running}");
    let payload = json!({
        "running": running,
        "url": core_rpc_url_from_env(),
    });
    Outcome::single_log(payload, "agent server status checked")
}
