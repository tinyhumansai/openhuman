//! Migration 5 -> 6: repair stale-zero `[http_request]` limits.
//!
//! Older builds could persist `http_request.timeout_secs = 0` and/or
//! `http_request.max_response_size = 0` (the plain numeric default before
//! the custom serde defaults landed). The network tools apply these
//! literally: `reqwest`'s `Duration::from_secs(0)` is an *instant* timeout
//! that fails every `web_fetch` / `http_request`, and a 0-byte cap
//! truncates every response body to nothing. `#[serde(default = …)]` only
//! fills *missing* keys, so a persisted `0` is taken as-is and silently
//! survives an app update — there is no way to express "blocked" or
//! "unlimited" with these zeros, only "broken".
//!
//! Coerce any `0` back to the schema default (30s / 1 MB). The tool
//! constructors also clamp `0` at the point of use (defence in depth); this
//! migration additionally repairs the persisted `config.toml` so it stops
//! shipping the misleading zeros to every consumer.

use crate::openhuman::config::{Config, HttpRequestConfig};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MigrationStats {
    pub timeout_repaired: bool,
    pub max_response_size_repaired: bool,
}

/// Returns `anyhow::Result` to match the shared migration-step signature that
/// [`crate::openhuman::migrations::run_pending`] dispatches (`Ok` → bump +
/// save, `Err` → log + retry next launch). This step is a pure in-memory
/// transform and is currently infallible, but keeping the `Result` lets a
/// future I/O-backed repair slot in without churning the runner or callers.
pub fn run(config: &mut Config) -> anyhow::Result<MigrationStats> {
    // Source the replacement values from the schema's own defaults so this
    // migration can't drift from `HttpRequestConfig`'s canonical defaults.
    let defaults = HttpRequestConfig::default();
    let mut stats = MigrationStats::default();

    if config.http_request.timeout_secs == 0 {
        config.http_request.timeout_secs = defaults.timeout_secs;
        stats.timeout_repaired = true;
    }
    if config.http_request.max_response_size == 0 {
        config.http_request.max_response_size = defaults.max_response_size;
        stats.max_response_size_repaired = true;
    }

    log::info!(
        "[migrations][repair-http-request-limits] done timeout_repaired={} \
         max_response_size_repaired={}",
        stats.timeout_repaired,
        stats.max_response_size_repaired
    );

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;

    #[test]
    fn repairs_zero_timeout_and_size() {
        let mut config = Config::default();
        config.http_request.timeout_secs = 0;
        config.http_request.max_response_size = 0;

        let stats = run(&mut config).expect("migration should succeed");

        let defaults = HttpRequestConfig::default();
        assert!(stats.timeout_repaired);
        assert!(stats.max_response_size_repaired);
        assert_eq!(config.http_request.timeout_secs, defaults.timeout_secs);
        assert_eq!(
            config.http_request.max_response_size,
            defaults.max_response_size
        );
        // The whole point: no zeros survive.
        assert_ne!(config.http_request.timeout_secs, 0);
        assert_ne!(config.http_request.max_response_size, 0);
    }

    #[test]
    fn repairs_only_the_zero_field() {
        let mut config = Config::default();
        config.http_request.timeout_secs = 0;
        config.http_request.max_response_size = 2_000_000;

        let stats = run(&mut config).expect("migration should succeed");

        assert!(stats.timeout_repaired);
        assert!(!stats.max_response_size_repaired);
        assert_ne!(config.http_request.timeout_secs, 0);
        assert_eq!(config.http_request.max_response_size, 2_000_000);
    }

    #[test]
    fn leaves_nonzero_values_untouched() {
        let mut config = Config::default();
        config.http_request.timeout_secs = 45;
        config.http_request.max_response_size = 3_000_000;

        let stats = run(&mut config).expect("migration should succeed");

        assert!(!stats.timeout_repaired);
        assert!(!stats.max_response_size_repaired);
        assert_eq!(config.http_request.timeout_secs, 45);
        assert_eq!(config.http_request.max_response_size, 3_000_000);
    }
}
