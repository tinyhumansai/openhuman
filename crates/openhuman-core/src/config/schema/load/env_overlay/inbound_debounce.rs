//! Env overrides for inbound message batching (`[inbound_debounce]`).

use crate::config::schema::load::env::parse_env_bool;
use crate::config::schema::load::env::EnvLookup;
use crate::config::schema::Config;

fn parse_ms(name: &str, raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.parse::<u64>() {
        Ok(ms) => Some(ms),
        Err(_) => {
            tracing::warn!(
                env = name,
                value = trimmed,
                "[config][inbound_debounce] expected milliseconds as an unsigned integer; ignored"
            );
            None
        }
    }
}

impl Config {
    pub(super) fn apply_inbound_debounce_env<E: EnvLookup + ?Sized>(&mut self, env: &E) {
        const ENABLED: &str = "OPENHUMAN_INBOUND_DEBOUNCE_ENABLED";
        if let Some(raw) = env.get(ENABLED) {
            if !raw.trim().is_empty() {
                if let Some(enabled) = parse_env_bool(ENABLED, &raw) {
                    self.inbound_debounce.enabled = enabled;
                }
            }
        }
        let targets: [(&str, &mut u64); 3] = [
            (
                "OPENHUMAN_INBOUND_DEBOUNCE_MS",
                &mut self.inbound_debounce.debounce_ms,
            ),
            (
                "OPENHUMAN_INBOUND_DEBOUNCE_MEDIA_MS",
                &mut self.inbound_debounce.media_debounce_ms,
            ),
            (
                "OPENHUMAN_INBOUND_DEBOUNCE_MAX_WAIT_MS",
                &mut self.inbound_debounce.max_wait_ms,
            ),
        ];
        for (name, target) in targets {
            if let Some(ms) = env.get(name).and_then(|raw| parse_ms(name, &raw)) {
                tracing::debug!(env = name, ms, "[config][inbound_debounce] env override");
                *target = ms;
            }
        }
    }
}
