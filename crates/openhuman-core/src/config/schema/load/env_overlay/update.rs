//! Env overrides for the auto-updater (`[update]`).

use crate::config::schema::load::env::parse_env_bool;
use crate::config::schema::load::env::EnvLookup;
use crate::config::schema::Config;
use crate::config::schema::UpdateRestartStrategy;

impl Config {
    pub(super) fn apply_update_env<E: EnvLookup + ?Sized>(&mut self, env: &E) {
        if let Some(flag) = env.get("OPENHUMAN_AUTO_UPDATE_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.update.enabled = true,
                "0" | "false" | "no" | "off" => self.update.enabled = false,
                _ => {}
            }
        }
        if let Some(val) = env.get("OPENHUMAN_AUTO_UPDATE_INTERVAL_MINUTES") {
            if let Ok(minutes) = val.trim().parse::<u32>() {
                self.update.interval_minutes = minutes;
            }
        }
        if let Some(raw) = env.get("OPENHUMAN_AUTO_UPDATE_RESTART_STRATEGY") {
            match raw.trim().to_ascii_lowercase().as_str() {
                "self_replace" | "self-replace" | "self" => {
                    self.update.restart_strategy = UpdateRestartStrategy::SelfReplace;
                }
                "supervisor" | "stage_only" | "stage-only" => {
                    self.update.restart_strategy = UpdateRestartStrategy::Supervisor;
                }
                other => {
                    tracing::warn!(
                        value = other,
                        "ignoring invalid OPENHUMAN_AUTO_UPDATE_RESTART_STRATEGY \
                         (valid: self_replace, supervisor)"
                    );
                }
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_AUTO_UPDATE_RPC_MUTATIONS_ENABLED") {
            if let Some(enabled) =
                parse_env_bool("OPENHUMAN_AUTO_UPDATE_RPC_MUTATIONS_ENABLED", &flag)
            {
                self.update.rpc_mutations_enabled = enabled;
            }
        }
    }
}
