//! Privacy Mode config operations (#4435).
//!
//! Mirrors the autonomy settings ops (`agent.rs`): a `get` that reads the
//! `[privacy]` block and a `set` that persists the new mode AND hot-swaps the
//! live `SecurityPolicy` so the inference chokepoint enforces the change
//! immediately, without a core restart.

use crate::openhuman::config::{Config, PrivacyMode};
use crate::rpc::RpcOutcome;

use super::loader::load_config_with_timeout;

/// Partial update for the `[privacy]` block. `None` leaves the mode unchanged.
#[derive(Debug, Clone, Default)]
pub struct PrivacySettingsPatch {
    /// `"local_only" | "standard" | "sensitive"` (case-insensitive; a few
    /// hyphen/space spellings are also accepted).
    pub mode: Option<String>,
}

/// Parse a user-supplied privacy-mode string into [`PrivacyMode`].
fn parse_privacy_mode(raw: &str) -> Result<PrivacyMode, String> {
    match raw
        .trim()
        .to_ascii_lowercase()
        .replace(['-', ' '], "_")
        .as_str()
    {
        "local_only" | "localonly" | "local" => Ok(PrivacyMode::LocalOnly),
        "standard" => Ok(PrivacyMode::Standard),
        "sensitive" => Ok(PrivacyMode::Sensitive),
        other => Err(format!(
            "invalid privacy mode '{other}' (expected local_only | standard | sensitive)"
        )),
    }
}

/// Serializable view of the current privacy mode for RPC responses.
fn privacy_mode_value(mode: PrivacyMode) -> serde_json::Value {
    serde_json::json!({ "mode": mode })
}

/// Returns the current `[privacy]` mode as `{ "mode": "<snake_case>" }`.
pub async fn get_privacy_mode() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    log::debug!(
        "[privacy][rpc] get_privacy_mode -> {:?}",
        config.privacy.mode
    );
    Ok(RpcOutcome::single_log(
        privacy_mode_value(config.privacy.mode),
        "privacy mode read",
    ))
}

/// Applies a privacy-mode update to `config`, persists it, and hot-swaps the
/// live `SecurityPolicy` so the inference chokepoint enforces the new mode
/// immediately. Returns `{ "mode": "<snake_case>" }`.
pub async fn apply_privacy_settings(
    config: &mut Config,
    update: PrivacySettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(raw) = update.mode {
        let mode = parse_privacy_mode(&raw)?;
        log::debug!(
            "[privacy][rpc] apply_privacy_settings: {:?} -> {:?}",
            config.privacy.mode,
            mode
        );
        config.privacy.mode = mode;
    }

    config.save().await.map_err(|e| e.to_string())?;

    // Hot-swap the live policy so enforcement takes effect without a restart.
    // `Err` here just means no session runtime is installed yet (e.g. CLI) — the
    // persisted value will be picked up on the next `install`; log and continue.
    match crate::openhuman::security::live_policy::reload_privacy(config.privacy.mode) {
        Ok(generation) => log::debug!(
            "[privacy][rpc] live policy reloaded to {:?} (generation={generation})",
            config.privacy.mode
        ),
        Err(e) => log::debug!(
            "[privacy][rpc] live policy not reloaded ({e}); persisted value applies on next install"
        ),
    }

    Ok(RpcOutcome::new(
        privacy_mode_value(config.privacy.mode),
        vec![format!(
            "privacy mode saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies the privacy-mode update, and saves it.
pub async fn load_and_apply_privacy_settings(
    update: PrivacySettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_privacy_settings(&mut config, update).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_privacy_mode_accepts_canonical_and_variants() {
        assert_eq!(
            parse_privacy_mode("local_only").unwrap(),
            PrivacyMode::LocalOnly
        );
        assert_eq!(
            parse_privacy_mode("Local-Only").unwrap(),
            PrivacyMode::LocalOnly
        );
        assert_eq!(
            parse_privacy_mode(" STANDARD ").unwrap(),
            PrivacyMode::Standard
        );
        assert_eq!(
            parse_privacy_mode("sensitive").unwrap(),
            PrivacyMode::Sensitive
        );
        assert!(parse_privacy_mode("bogus").is_err());
    }

    #[test]
    fn privacy_mode_value_shape() {
        assert_eq!(
            privacy_mode_value(PrivacyMode::LocalOnly),
            serde_json::json!({ "mode": "local_only" })
        );
    }
}
