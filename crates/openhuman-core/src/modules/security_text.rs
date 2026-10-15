//! Host client for the module's text surfaces: `Redact`, `ScanPrompt` and
//! `ScanToolDefinition`.
//!
//! The module owns every redaction and scan decision. This file owns transport,
//! a bounded cache for hot logging, and the failure rule: when the module is
//! unavailable, faulted, times out or answers something malformed, content is
//! **suppressed** ([`SUPPRESSED`]), never returned unredacted. The only text the
//! host may print without the module is the fixed boot floor
//! (`core::log_redaction`, tinysecurity ADR 7).
//!
//! A scan verdict is context for the caller, not authorization: nothing here
//! grants an effect.

use std::collections::HashMap;
use std::sync::Mutex;

use sha2::{Digest, Sha256};
use tinysecurity_bus::{
    names, RedactRequest, RedactResponse, ScanPromptRequest, ScanToolDefinitionRequest,
};
pub(crate) use tinysecurity_bus::{RedactMode, ScanResponse, ScanVerdict};

use crate::config::Config;
pub(crate) use crate::core::log_redaction::SUPPRESSED;

const CACHE_CAPACITY: usize = 2048;
/// Larger inputs are not cached (one-off payloads would only evict hot lines).
const CACHE_MAX_INPUT_BYTES: usize = 8 * 1024;

/// Bounded `(mode, text digest) -> redacted text` cache. Keys are digests so the
/// cache never retains a secret in the clear; values are module output.
#[derive(Default)]
pub(crate) struct RedactCache {
    entries: HashMap<String, String>,
}

impl RedactCache {
    fn key(mode: &RedactMode, text: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!("{mode:?}").as_bytes());
        hasher.update([0x1f]);
        hasher.update(text.as_bytes());
        hex::encode(hasher.finalize())
    }

    pub(crate) fn get(&self, mode: &RedactMode, text: &str) -> Option<String> {
        self.entries.get(&Self::key(mode, text)).cloned()
    }

    pub(crate) fn put(&mut self, mode: &RedactMode, text: &str, redacted: &str) -> bool {
        if text.len() > CACHE_MAX_INPUT_BYTES {
            return false;
        }
        if self.entries.len() >= CACHE_CAPACITY {
            self.entries.clear();
        }
        self.entries
            .insert(Self::key(mode, text), redacted.to_string());
        true
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}

fn cache() -> &'static Mutex<RedactCache> {
    static CACHE: std::sync::OnceLock<Mutex<RedactCache>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(RedactCache::default()))
}

/// Drop every cached redaction: the module's redactor configuration (exact
/// secrets, generation) may have changed.
pub(crate) fn flush_cache() {
    if let Ok(mut cache) = cache().lock() {
        cache.clear();
    }
}

/// Redact `text` on `mode` through an explicit proxy and cache.
pub(crate) async fn redact_on(
    proxy: &tinybus::Proxy,
    cache: &Mutex<RedactCache>,
    mode: RedactMode,
    text: &str,
) -> Result<String, String> {
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(&mode, text)) {
        return Ok(hit);
    }
    let response: RedactResponse = proxy
        .call_confidential(
            names::methods::REDACT,
            (RedactRequest {
                text: text.to_string(),
                mode: mode.clone(),
            },),
        )
        .await
        .map_err(|_| "TinySecurity Redact failed")?;
    if let Ok(mut cache) = cache.lock() {
        cache.put(&mode, text, &response.text);
    }
    Ok(response.text)
}

/// Redact through the live module. Errors are returned so callers that must
/// deny (approvals, egress, prompts) can; log-path callers use
/// [`redact_or_suppress`].
pub(crate) async fn redact(
    config: &Config,
    mode: RedactMode,
    text: &str,
) -> Result<String, String> {
    super::require_enabled(config)?;
    let proxy = super::scoped_client::module_proxy(config).await?;
    redact_on(&proxy, cache(), mode, text).await
}

/// Redact, or return [`SUPPRESSED`] when the module cannot answer. The original
/// text is never returned on failure.
pub(crate) async fn redact_or_suppress(config: &Config, mode: RedactMode, text: &str) -> String {
    match redact(config, mode, text).await {
        Ok(redacted) => redacted,
        Err(error) => {
            log::debug!("[security][redact] module unavailable, suppressing: {error}");
            SUPPRESSED.to_string()
        }
    }
}

/// Redact many texts on one mode concurrently (one cached call each).
pub(crate) async fn redact_batch(
    config: &Config,
    mode: RedactMode,
    texts: &[String],
) -> Vec<String> {
    futures::future::join_all(
        texts
            .iter()
            .map(|text| redact_or_suppress(config, mode.clone(), text)),
    )
    .await
}

/// Scan a prompt for injection. The verdict is context, not authorization.
pub(crate) async fn scan_prompt(config: &Config, text: &str) -> Result<ScanResponse, String> {
    super::require_enabled(config)?;
    let proxy = super::scoped_client::module_proxy(config).await?;
    scan_prompt_on(&proxy, text).await
}

pub(crate) async fn scan_prompt_on(
    proxy: &tinybus::Proxy,
    text: &str,
) -> Result<ScanResponse, String> {
    proxy
        .call_confidential(
            names::methods::SCAN_PROMPT,
            (ScanPromptRequest {
                text: text.to_string(),
            },),
        )
        .await
        .map_err(|_| "TinySecurity ScanPrompt failed".into())
}

/// Scan a tool definition (name plus JSON schema) for injected instructions.
pub(crate) async fn scan_tool_definition(
    config: &Config,
    name: &str,
    definition: &serde_json::Value,
) -> Result<ScanResponse, String> {
    super::require_enabled(config)?;
    let proxy = super::scoped_client::module_proxy(config).await?;
    scan_tool_definition_on(&proxy, name, definition).await
}

pub(crate) async fn scan_tool_definition_on(
    proxy: &tinybus::Proxy,
    name: &str,
    definition: &serde_json::Value,
) -> Result<ScanResponse, String> {
    proxy
        .call_confidential(
            names::methods::SCAN_TOOL_DEFINITION,
            (ScanToolDefinitionRequest {
                name: name.to_string(),
                definition_json: definition.to_string(),
            },),
        )
        .await
        .map_err(|_| "TinySecurity ScanToolDefinition failed".into())
}

/// A scan the caller could not obtain is treated as suspicious, never clean.
pub(crate) fn verdict_or_suspicious(result: Result<ScanResponse, String>) -> ScanVerdict {
    match result {
        Ok(response) => response.verdict,
        Err(_) => ScanVerdict::Suspicious,
    }
}

// ---------------------------------------------------------------------------
// Synchronous log/Sentry bridge
// ---------------------------------------------------------------------------

/// Longest a synchronous log call waits for the module before the floor result
/// stands.
const LOG_BRIDGE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

struct LogBridge {
    config: Config,
}

static LOG_BRIDGE: std::sync::OnceLock<LogBridge> = std::sync::OnceLock::new();

thread_local! {
    static IN_BRIDGE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Redact from a synchronous caller through the shared seam
/// ([`super::sync_seam`]): bounded, never `block_on` on a worker thread, and
/// `Err` (never the original text) when the module cannot answer.
pub(crate) fn redact_blocking(
    config: &Config,
    mode: RedactMode,
    text: &str,
    timeout: std::time::Duration,
) -> Result<String, String> {
    if let Some(hit) = cache().lock().ok().and_then(|c| c.get(&mode, text)) {
        return Ok(hit);
    }
    let config = config.clone();
    let text = text.to_string();
    super::sync_seam::run(timeout, async move { redact(&config, mode, &text).await })
        .map_err(String::from)
}

/// Arm the module-backed second pass for `core::log_redaction::scrub_secrets`
/// (Sentry `before_send`, error logs). Called once the module answered; a no-op
/// outside a tokio runtime or when already armed.
pub(crate) fn arm_log_scrubber(config: &Config) {
    if !super::sync_seam::arm_current() {
        return;
    }
    if LOG_BRIDGE
        .set(LogBridge {
            config: config.clone(),
        })
        .is_ok()
    {
        crate::core::log_redaction::install_module_scrubber(bridge_scrub);
    }
}

/// Cache first, then at most [`LOG_BRIDGE_TIMEOUT`] on the module. `None` leaves
/// the caller with the floor result.
fn bridge_scrub(text: &str) -> Option<String> {
    scrub_with_bridge(LOG_BRIDGE.get(), text)
}

/// [`bridge_scrub`] for an explicit bridge (`None` when it was never armed).
fn scrub_with_bridge(bridge: Option<&LogBridge>, text: &str) -> Option<String> {
    let bridge = bridge?;
    // The module call may itself log; never recurse into the bridge.
    if IN_BRIDGE.with(|flag| flag.replace(true)) {
        return None;
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            IN_BRIDGE.with(|flag| flag.set(false));
        }
    }
    let _reset = Reset;
    match redact_blocking(&bridge.config, RedactMode::Text, text, LOG_BRIDGE_TIMEOUT) {
        Ok(redacted) => Some(redacted),
        Err(_) => {
            log::debug!("[security][floor] module redact unavailable; floor result stands");
            None
        }
    }
}

#[cfg(test)]
#[path = "security_text_tests.rs"]
mod tests;
