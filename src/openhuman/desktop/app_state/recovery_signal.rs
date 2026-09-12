//! Process-lifetime latch recording whether the config loader recovered a
//! corrupted `config.toml` during this session (#5167).
//!
//! The loader (`config::schema::load`) heals a corrupt config on the *first*
//! read of the process by renaming it to `.corrupted.<ts>` and resetting to
//! defaults, so the per-load [`Config::recovered_from_corruption`] flag is only
//! `true` on that first load and `false` on every subsequent read of the
//! now-healed file. The frontend, however, polls `app_state_snapshot` and may
//! not do so until after the heal — by which point the flag would already read
//! `false`. This latch bridges that gap: `bootstrap_core_runtime` sets it once
//! from the boot config, and every later snapshot reports it, so the notice
//! surfaces even though the underlying config is already healthy.
//!
//! `app_state_snapshot` also latches from each poll's freshly-loaded config, so
//! corruption that first appears *after* boot (the running config.toml is
//! edited/damaged and healed on a later reload) is caught too, not just the
//! boot-time case.
//!
//! [`Config::recovered_from_corruption`]:
//! crate::openhuman::config::Config::recovered_from_corruption

use std::sync::atomic::{AtomicBool, Ordering};

use crate::openhuman::config::Config;

/// `true` once the config loader recovered a corrupted `config.toml` this
/// process lifetime. Never reset in production (a recovery is a one-time,
/// session-scoped fact); tests clear it via [`reset_for_tests`].
static CONFIG_RECOVERED: AtomicBool = AtomicBool::new(false);

/// Record that config-corruption recovery happened this session.
///
/// Returns `true` only on the first call this process (the `false`→`true`
/// transition); subsequent calls return `false`. Lets a caller invoked on every
/// snapshot poll act (e.g. log) exactly once. Idempotent and safe to call
/// repeatedly.
fn mark_config_recovered() -> bool {
    !CONFIG_RECOVERED.swap(true, Ordering::Relaxed)
}

/// Latch the session recovery signal from a freshly-loaded config.
///
/// Called from `bootstrap_core_runtime` with the boot config returned by
/// `Config::load_or_init`, and from every `app_state_snapshot` poll with the
/// config that poll just loaded — whichever load's `recovered_from_corruption`
/// is authoritative wins, so both boot-time and mid-session recovery latch.
/// No-op when the config loaded cleanly.
///
/// Logs at warn **once per process**, gated on the latch's `false`→`true`
/// transition rather than on `recovered_from_corruption` itself. That matters
/// because this runs on every snapshot poll (~every few seconds): when a corrupt
/// config cannot be healed — `impl_load` skips the heal if the rename to
/// `.corrupted.<ts>` fails (e.g. a Windows sharing violation, os error 32) and
/// retries on the next load — every poll re-recovers, and an ungated warn would
/// emit indefinitely. It is a Sentry breadcrumb, not an event (the read failure
/// it stems from is expected user-environment state, #5167).
pub fn latch_from_config(config: &Config) {
    if !config.recovered_from_corruption {
        return;
    }
    if mark_config_recovered() {
        tracing::warn!(
            "[app_state] config.toml was unreadable/corrupt and was recovered \
             (restored from .bak or reset to defaults; previous file kept as \
             .corrupted.<ts>); surfacing a user notice (#5167)"
        );
    }
}

/// Whether config-corruption recovery happened this session. Read by
/// `app_state_snapshot` so the frontend can raise a one-shot user notice.
pub fn config_recovered_this_session() -> bool {
    CONFIG_RECOVERED.load(Ordering::Relaxed)
}

/// Reset the latch. Test-only: the static is process-global, so a test that
/// asserts the un-recovered default must clear state a prior test may have set.
#[cfg(test)]
pub fn reset_for_tests() {
    CONFIG_RECOVERED.store(false, Ordering::Relaxed);
}

#[cfg(test)]
#[path = "recovery_signal_tests.rs"]
mod tests;
