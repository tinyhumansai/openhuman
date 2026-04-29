//! Prompt and response dump utilities for KV-cache debugging.
//!
//! When `OPENHUMAN_PROMPT_DUMP_DIR` is set, both the outbound request payload
//! and the inbound response body are written to timestamped files under that
//! directory. Best-effort: failures are logged and swallowed so a dump outage
//! never breaks inference.

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic sequence so multiple requests in the same millisecond sort
/// deterministically in the dump directory.
pub(crate) static PROMPT_DUMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Atomically reserve the next dump sequence number. This is the single
/// source of truth for seq allocation — both the prompt dump and its
/// paired response dump must use the value returned here. A non-atomic
/// peek-then-increment split would race under concurrent requests (two
/// callers could reserve the same seq or correlate a request/response
/// pair across different turns).
pub(crate) fn reserve_dump_seq() -> u64 {
    PROMPT_DUMP_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// When `OPENHUMAN_PROMPT_DUMP_DIR` is set, write `body` (the exact JSON
/// payload we're about to POST to the provider) to a timestamped file
/// under that directory. Best-effort: failures are logged and swallowed
/// so a dump outage never breaks inference.
///
/// Intended for KV-cache debugging — diff consecutive turns to see which
/// bytes of the prefix drifted and broke the cache hit.
pub(crate) fn dump_prompt_if_enabled<T: Serialize>(
    provider: &str,
    model: &str,
    seq: u64,
    body: &T,
) {
    let Ok(dir) = std::env::var("OPENHUMAN_PROMPT_DUMP_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    if let Err(err) = std::fs::create_dir_all(&dir) {
        log::warn!(
            "[prompt_dump] failed to create dir {}: {err}",
            dir.display()
        );
        return;
    }
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let safe_model: String = model
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let filename = format!("{ts}_{seq:06}_{provider}_{safe_model}.json");
    let path = dir.join(filename);
    match serde_json::to_vec_pretty(body) {
        Ok(bytes) => {
            if let Err(err) = std::fs::write(&path, &bytes) {
                log::warn!("[prompt_dump] write failed {}: {err}", path.display());
            } else {
                log::debug!(
                    "[prompt_dump] wrote {} bytes -> {}",
                    bytes.len(),
                    path.display()
                );
            }
        }
        Err(err) => {
            log::warn!("[prompt_dump] serialize failed: {err}");
        }
    }
}

/// Write raw response bytes to the dump dir paired with the most-recent
/// prompt file (same `seq` prefix, `.response.json` suffix). `seq` must
/// be the value reserved via `reserve_dump_seq` and passed to
/// `dump_prompt_if_enabled` so request/response files sort next to
/// each other.
pub(crate) fn dump_response_if_enabled(provider: &str, model: &str, seq: u64, bytes: &[u8]) {
    let Ok(dir) = std::env::var("OPENHUMAN_PROMPT_DUMP_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    if let Err(err) = std::fs::create_dir_all(&dir) {
        log::warn!(
            "[prompt_dump] failed to create dir {}: {err}",
            dir.display()
        );
        return;
    }
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let safe_model: String = model
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let filename = format!("{ts}_{seq:06}_{provider}_{safe_model}.response.json");
    let path = dir.join(filename);
    // Re-pretty-print if it parses as JSON so diffs are stable; otherwise
    // write raw bytes (SSE fragments, error HTML, etc).
    let payload: Vec<u8> = match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(v) => serde_json::to_vec_pretty(&v).unwrap_or_else(|_| bytes.to_vec()),
        Err(_) => bytes.to_vec(),
    };
    if let Err(err) = std::fs::write(&path, &payload) {
        log::warn!(
            "[prompt_dump] response write failed {}: {err}",
            path.display()
        );
    } else {
        log::debug!(
            "[prompt_dump] wrote response {} bytes -> {}",
            payload.len(),
            path.display()
        );
    }
}
