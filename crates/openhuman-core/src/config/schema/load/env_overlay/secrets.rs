//! Env overrides for config-secret handling (`[secrets]`).

use crate::config::schema::load::env::parse_env_bool;
use crate::config::schema::load::env::EnvLookup;
use crate::config::schema::Config;

/// Env override for [`crate::config::schema::SecretsConfig::unattended_unlock`].
pub(crate) const UNATTENDED_UNLOCK_ENV: &str = "OPENHUMAN_SECRETS_UNATTENDED_UNLOCK";

impl Config {
    pub(super) fn apply_secrets_env<E: EnvLookup + ?Sized>(&mut self, env: &E) {
        let Some(raw) = env
            .get(UNATTENDED_UNLOCK_ENV)
            .filter(|raw| !raw.trim().is_empty())
        else {
            return;
        };
        if let Some(value) = parse_env_bool(UNATTENDED_UNLOCK_ENV, &raw) {
            tracing::debug!(
                unattended_unlock = value,
                "[config][secrets] {UNATTENDED_UNLOCK_ENV} applied"
            );
            self.secrets.unattended_unlock = value;
            self.record_env_override(env, "secrets.unattended_unlock", &[UNATTENDED_UNLOCK_ENV]);
        }
    }
}
