//! Env overrides for `[components]`: `OPENHUMAN_COMPONENT_<NAME>=0|1`.

use crate::config::schema::components::COMPONENT_NAMES;
use crate::config::schema::load::env::EnvLookup;
use crate::config::schema::Config;

/// Parse a `0|1|true|false|on|off|yes|no` flag; `None` for anything else.
pub(crate) fn parse_flag(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

impl Config {
    pub(super) fn apply_components_env<E: EnvLookup + ?Sized>(&mut self, env: &E) {
        // Legacy listener kill switch, folded into `components.channels` for
        // one release. An explicit `OPENHUMAN_COMPONENT_CHANNELS` below wins.
        if env
            .get("OPENHUMAN_DISABLE_CHANNEL_LISTENERS")
            .and_then(|raw| parse_flag(&raw))
            == Some(true)
        {
            tracing::debug!(
                "[components] OPENHUMAN_DISABLE_CHANNEL_LISTENERS folds to channels=false"
            );
            self.components.set("channels", false);
        }
        for name in COMPONENT_NAMES {
            let key = format!("OPENHUMAN_COMPONENT_{}", name.to_ascii_uppercase());
            let Some(raw) = env.get(&key) else { continue };
            match parse_flag(&raw) {
                Some(value) => {
                    tracing::debug!("[components] env override {key}={value}");
                    self.components.set(name, value);
                }
                None => tracing::warn!("[components] ignoring invalid {key} (use 0 or 1)"),
            }
        }
    }
}
