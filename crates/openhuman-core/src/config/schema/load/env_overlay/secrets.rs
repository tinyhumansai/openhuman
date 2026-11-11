//! Env overrides for config-secret handling (`[secrets]`).

use crate::config::schema::load::env::parse_env_bool;
use crate::config::schema::load::env::EnvLookup;
use crate::config::schema::Config;

pub(crate) use crate::config::schema::tools::integrations::SECRETS_UNATTENDED_UNLOCK_ENV as UNATTENDED_UNLOCK_ENV;

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

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod tests;
