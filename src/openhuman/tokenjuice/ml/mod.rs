//! TokenJuice ML plain-text compressor ("Kompress").
//!
//! Plain text has no structural skeleton to exploit, so high-quality
//! compression needs a learned model (ModernBERT token/sentence salience). That
//! runs inside the shared [`crate::openhuman::runtime_python_server`] as the
//! `kompress` backend — this module is just the thin Rust entry the
//! [`crate::openhuman::tokenjuice::compressors::ml_text`] compressor calls.
//!
//! Opt-in at runtime via `config.tokenjuice.ml_compression_enabled` (default
//! off) — there is no build-time feature gate, since torch is provisioned at
//! runtime (pip), never linked. Degrades gracefully (`Ok(None)` / `Err`) when
//! the flag is off, the runtime python server is unavailable, or the input is
//! too large — the agent loop never fails because ML compression is missing.

use std::sync::{OnceLock, RwLock};

use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::tokenjuice::types::CompressOptions;

/// Global config snapshot the Kompress backend runs against. Held behind a
/// `RwLock` (not a `OnceLock`) so a live settings update — e.g. toggling
/// `ml_compression_enabled` on from Settings — is picked up without a restart.
fn config_cell() -> &'static RwLock<Option<Config>> {
    static CONFIG: OnceLock<RwLock<Option<Config>>> = OnceLock::new();
    CONFIG.get_or_init(|| RwLock::new(None))
}

/// Install (or replace) the config snapshot. Called at startup and on every
/// `tokenjuice.settings_update` so the runtime sees current values.
pub fn configure(config: Config) {
    *config_cell().write().unwrap_or_else(|p| p.into_inner()) = Some(config);
}

/// Compress `text` via the Kompress backend of the runtime python server.
///
/// Returns `Ok(Some(compacted))` on a useful result, `Ok(None)` when the flag
/// is off / input too large / output wouldn't help, and `Err` when the backend
/// is unavailable (caller degrades to a native compressor).
pub async fn compress(text: &str, _opts: &CompressOptions) -> Result<Option<String>> {
    // Snapshot the current config under the read lock (live-updated by
    // `configure` on settings changes), then release it before the await.
    let config = {
        let guard = config_cell().read().unwrap_or_else(|p| p.into_inner());
        match guard.as_ref() {
            Some(c) => c.clone(),
            None => anyhow::bail!("tokenjuice ml not configured"),
        }
    };
    let tj = &config.tokenjuice;
    if !tj.ml_compression_enabled {
        return Ok(None);
    }
    if text.len() > tj.ml_max_input_chars {
        // Too large for the model — let a native compressor handle it.
        return Ok(None);
    }

    let resp = crate::openhuman::runtime_python_server::request_kompress(&config, text).await?;
    if resp.compressed_text.is_empty() || resp.compressed_text.len() >= text.len() {
        return Ok(None);
    }
    log::debug!(
        "[tokenjuice::ml] kompress {} -> {} chars",
        resp.input_chars,
        resp.output_chars
    );
    Ok(Some(resp.compressed_text))
}
