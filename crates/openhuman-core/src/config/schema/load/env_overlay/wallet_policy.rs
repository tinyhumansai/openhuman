//! Env overrides for the native-wallet spend policy (`[wallet_policy]`).

use crate::config::schema::load::env::parse_env_bool;
use crate::config::schema::load::env::EnvLookup;
use crate::config::schema::{Config, WalletAssetCap};

impl Config {
    pub(super) fn apply_wallet_policy_env<E: EnvLookup + ?Sized>(&mut self, env: &E) {
        for (name, target) in [
            (
                "OPENHUMAN_WALLET_POLICY_AUTOMATION_ENABLED",
                &mut self.wallet_policy.automation_enabled,
            ),
            (
                "OPENHUMAN_WALLET_POLICY_APPLY_TO_INTERACTIVE",
                &mut self.wallet_policy.apply_to_interactive,
            ),
        ] {
            if let Some(raw) = env.get(name).filter(|raw| !raw.trim().is_empty()) {
                if let Some(value) = parse_env_bool(name, &raw) {
                    *target = value;
                }
            }
        }
        const ASSETS: &str = "OPENHUMAN_WALLET_POLICY_ASSETS";
        if let Some(raw) = env.get(ASSETS).filter(|raw| !raw.trim().is_empty()) {
            match serde_json::from_str::<Vec<WalletAssetCap>>(raw.trim()) {
                Ok(assets) => {
                    tracing::debug!(
                        count = assets.len(),
                        "[config][wallet_policy] asset caps from env"
                    );
                    self.wallet_policy.assets = assets;
                }
                // A malformed override must not silently widen the policy: keep
                // the configured caps and say so.
                Err(error) => tracing::warn!(
                    env = ASSETS,
                    %error,
                    "[config][wallet_policy] expected a JSON array of asset caps; ignored"
                ),
            }
        }
    }
}
