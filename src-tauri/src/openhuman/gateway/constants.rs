//! Gateway constants and key helpers.

use crate::openhuman::channels::traits::ChannelMessage;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Maximum request body size (64KB) — prevents memory exhaustion.
pub const MAX_BODY_SIZE: usize = 65_536;
/// Request timeout (30s) — prevents slow-loris attacks.
pub const REQUEST_TIMEOUT_SECS: u64 = 30;
/// Sliding window used by gateway rate limiting.
pub const RATE_LIMIT_WINDOW_SECS: u64 = 60;
/// Fallback max distinct client keys tracked in gateway rate limiter.
pub const RATE_LIMIT_MAX_KEYS_DEFAULT: usize = 10_000;
/// Fallback max distinct idempotency keys retained in gateway memory.
pub const IDEMPOTENCY_MAX_KEYS_DEFAULT: usize = 10_000;
/// How often the rate limiter sweeps stale IP entries from its map.
pub const RATE_LIMITER_SWEEP_INTERVAL_SECS: u64 = 300; // 5 minutes

/// Unique memory key for webhook messages.
pub fn webhook_memory_key() -> String {
    format!("webhook_msg_{}", Uuid::new_v4())
}

/// Memory key for WhatsApp messages.
pub fn whatsapp_memory_key(msg: &ChannelMessage) -> String {
    format!("whatsapp_{}_{}", msg.sender, msg.id)
}

/// Memory key for Linq messages.
pub fn linq_memory_key(msg: &ChannelMessage) -> String {
    format!("linq_{}_{}", msg.sender, msg.id)
}

/// Hash a webhook secret using SHA-256 (hex-encoded).
pub fn hash_webhook_secret(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex::encode(digest)
}
