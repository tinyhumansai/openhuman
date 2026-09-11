//! Migration 9 → 10: retire the removed local whisper.cpp STT provider.
//!
//! ## The problem
//!
//! Older builds shipped a bundled whisper.cpp speech-to-text engine — an
//! in-process `whisper-rs` context plus a `whisper-cli` subprocess fallback —
//! selected with `stt_provider = "whisper"`. Both are gone: the engine, its
//! model/binary downloader, and the `whisper-rs` dependency were deleted in
//! favour of hosted engines chosen through `voice_server.stt_engine`.
//!
//! A user who picked local STT keeps `stt_provider = "whisper"` in their
//! persisted `config.toml`. `"whisper"` is no longer a sentinel in
//! [`crate::openhuman::voice::factory::create_stt_provider`], so it falls
//! through to the third-party slug lookup and errors with "no voice provider
//! with slug 'whisper'" — every dictation attempt fails, with an error naming a
//! provider the user never configured.
//!
//! ## What this migration does
//!
//! A pure, idempotent mutation of the persisted `Config`: any `stt_provider`
//! naming the removed local engine (`"whisper"` or `"local"`, in either the
//! top-level field or the legacy `local_ai` one) is rewritten to `"cloud"`,
//! which defers to `voice_server.stt_engine` — the hosted backend proxy by
//! default. Nothing else is touched: a config already on `"cloud"`, on a
//! third-party slug, or with `stt_provider = None` comes through unchanged, so
//! this can only ever move a broken config to a working one.
//!
//! The engine field itself is deliberately **not** set here. Its serde default
//! is already `Backend`, and writing an explicit value would overwrite a choice
//! a user made in a build that shipped both.
//!
//! ## Behaviour
//!
//! - `run` is a **pure, synchronous** in-memory mutation; the caller
//!   ([`super::run_pending`]) persists via `Config::save()` and bumps
//!   `schema_version`.
//! - Idempotent: after the rewrite neither field names the local engine, so a
//!   second run is a no-op.
//! - Never touches keys, secrets, or any other config field.

use crate::openhuman::config::Config;

/// Routing strings that selected the removed local whisper.cpp engine.
const REMOVED_LOCAL_PROVIDERS: &[&str] = &["whisper", "local"];

/// Rewrite target. Not a hosted provider name — it is the routing grammar's
/// "use the configured engine" sentinel, so the user lands on whatever
/// `voice_server.stt_engine` says (the backend proxy unless they changed it).
const REPLACEMENT: &str = "cloud";

fn names_removed_local_engine(value: &str) -> bool {
    let trimmed = value.trim();
    REMOVED_LOCAL_PROVIDERS
        .iter()
        .any(|removed| trimmed.eq_ignore_ascii_case(removed))
}

/// Counters returned by [`run`] for diagnostics. Logged at INFO once per run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MigrationStats {
    /// Whether the top-level `stt_provider` was rewritten.
    pub stt_provider_migrated: bool,
    /// Whether the legacy `local_ai.stt_provider` was rewritten.
    pub legacy_stt_provider_migrated: bool,
}

/// Rewrite a persisted local-whisper STT selection to the hosted default.
///
/// Synchronous — pure config mutation, no I/O. Caller persists via
/// `Config::save()` once `schema_version` is also bumped.
///
/// Returns `anyhow::Result` for uniformity with the other migration steps in
/// [`super`]; this pass has no fallible operations and always returns `Ok`.
pub fn run(config: &mut Config) -> anyhow::Result<MigrationStats> {
    let mut stats = MigrationStats::default();

    if let Some(provider) = config.stt_provider.as_deref() {
        if names_removed_local_engine(provider) {
            log::info!(
                "[migrations][retire-local-whisper] stt_provider \"{provider}\" -> \
                 \"{REPLACEMENT}\" (defers to voice_server.stt_engine)"
            );
            config.stt_provider = Some(REPLACEMENT.to_string());
            stats.stt_provider_migrated = true;
        }
    }

    if names_removed_local_engine(&config.local_ai.stt_provider) {
        log::info!(
            "[migrations][retire-local-whisper] local_ai.stt_provider \"{}\" -> \"{REPLACEMENT}\"",
            config.local_ai.stt_provider
        );
        config.local_ai.stt_provider = REPLACEMENT.to_string();
        stats.legacy_stt_provider_migrated = true;
    }

    if stats == MigrationStats::default() {
        log::debug!(
            "[migrations][retire-local-whisper] no local-whisper STT selection found — \
             nothing to do"
        );
    }

    Ok(stats)
}

#[cfg(test)]
#[path = "retire_local_whisper_stt_tests.rs"]
mod tests;
