// First-connect authentication for channels (e.g. Telegram) that support operator pairing.
//
// A one-time pairing code can be shown to the operator; successful pairing issues
// a bearer token. Tokens can be persisted in config so restarts don't require
// re-pairing.

use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

/// Maximum failed pairing attempts before lockout.
const MAX_PAIR_ATTEMPTS: u32 = 5;
/// Lockout duration after too many failed pairing attempts.
const PAIR_LOCKOUT_SECS: u64 = 300; // 5 minutes

/// Manages pairing state for channels that use bearer-token auth after pairing.
///
/// Bearer tokens are stored as SHA-256 hashes to prevent plaintext exposure
/// in config files. When a new token is generated, the plaintext is returned
/// to the client once, and only the hash is retained.
// TODO: I've just made this work with parking_lot but it should use either flume or tokio's async mutexes
#[derive(Debug, Clone)]
pub struct PairingGuard {
    /// Whether pairing is required at all.
    require_pairing: bool,
    /// One-time pairing code (generated on startup, consumed on first pair).
    pairing_code: Arc<Mutex<Option<String>>>,
    /// Set of SHA-256 hashed bearer tokens (persisted across restarts).
    paired_tokens: Arc<Mutex<HashSet<String>>>,
    /// Brute-force protection: failed attempt counter + lockout time.
    failed_attempts: Arc<Mutex<(u32, Option<Instant>)>>,
}

impl PairingGuard {
    /// Create a new pairing guard.
    ///
    /// If `require_pairing` is true and no tokens exist yet, a fresh
    /// pairing code is generated and returned via `pairing_code()`.
    ///
    /// Existing tokens are accepted in both forms:
    /// - Plaintext (`zc_...`): hashed on load for backward compatibility
    /// - Already hashed (64-char hex): stored as-is
    pub fn new(require_pairing: bool, existing_tokens: &[String]) -> Self {
        let tokens: HashSet<String> = existing_tokens
            .iter()
            .map(|t| {
                if is_token_hash(t) {
                    t.clone()
                } else {
                    hash_token(t)
                }
            })
            .collect();
        let code = if require_pairing && tokens.is_empty() {
            Some(generate_code())
        } else {
            None
        };
        log::info!(
            "[openhuman:pairing] Guard created: require_pairing={}, existing_tokens={}, code_generated={}",
            require_pairing,
            tokens.len(),
            code.is_some()
        );
        Self {
            require_pairing,
            pairing_code: Arc::new(Mutex::new(code)),
            paired_tokens: Arc::new(Mutex::new(tokens)),
            failed_attempts: Arc::new(Mutex::new((0, None))),
        }
    }

    /// The one-time pairing code (only set when no tokens exist yet).
    pub fn pairing_code(&self) -> Option<String> {
        self.pairing_code.lock().clone()
    }

    /// Whether pairing is required at all.
    pub fn require_pairing(&self) -> bool {
        self.require_pairing
    }

    fn try_pair_blocking(&self, code: &str) -> Result<Option<String>, u64> {
        // Check brute force lockout
        {
            let attempts = self.failed_attempts.lock();
            if let (count, Some(locked_at)) = &*attempts {
                if *count >= MAX_PAIR_ATTEMPTS {
                    let elapsed = locked_at.elapsed().as_secs();
                    if elapsed < PAIR_LOCKOUT_SECS {
                        log::warn!(
                            "[openhuman:pairing] Pairing locked out: {} failed attempts, {}s remaining",
                            count,
                            PAIR_LOCKOUT_SECS - elapsed
                        );
                        return Err(PAIR_LOCKOUT_SECS - elapsed);
                    }
                }
            }
        }

        {
            let mut pairing_code = self.pairing_code.lock();
            if let Some(ref expected) = *pairing_code {
                if constant_time_eq(code.trim(), expected.trim()) {
                    // Reset failed attempts on success
                    {
                        let mut attempts = self.failed_attempts.lock();
                        *attempts = (0, None);
                    }
                    let token = generate_token();
                    let mut tokens = self.paired_tokens.lock();
                    tokens.insert(hash_token(&token));

                    // Consume the pairing code so it cannot be reused
                    *pairing_code = None;

                    log::info!("[openhuman:pairing] Pairing successful, token issued");
                    return Ok(Some(token));
                }
            }
        }

        // Increment failed attempts
        {
            let mut attempts = self.failed_attempts.lock();
            attempts.0 += 1;
            log::warn!(
                "[openhuman:pairing] Pairing attempt failed ({}/{})",
                attempts.0,
                MAX_PAIR_ATTEMPTS
            );
            if attempts.0 >= MAX_PAIR_ATTEMPTS {
                attempts.1 = Some(Instant::now());
                log::warn!("[openhuman:pairing] Max attempts reached, lockout activated");
            }
        }

        Ok(None)
    }

    /// Attempt to pair with the given code. Returns a bearer token on success.
    /// Returns `Err(lockout_seconds)` if locked out due to brute force.
    pub async fn try_pair(&self, code: &str) -> Result<Option<String>, u64> {
        let this = self.clone();
        let code = code.to_string();
        // TODO: make this function the main one without spawning a task
        let handle = tokio::task::spawn_blocking(move || this.try_pair_blocking(&code));

        handle
            .await
            .expect("failed to spawn blocking task this should not happen")
    }

    /// Check if a bearer token is valid (compares against stored hashes).
    pub fn is_authenticated(&self, token: &str) -> bool {
        if !self.require_pairing {
            return true;
        }
        let hashed = hash_token(token);
        let tokens = self.paired_tokens.lock();
        tokens.contains(&hashed)
    }

    /// Returns true if pairing is satisfied (has at least one token).
    pub fn is_paired(&self) -> bool {
        let tokens = self.paired_tokens.lock();
        !tokens.is_empty()
    }

    /// Get all paired token hashes (for persisting to config).
    pub fn tokens(&self) -> Vec<String> {
        let tokens = self.paired_tokens.lock();
        tokens.iter().cloned().collect()
    }
}

/// Generate a 6-digit numeric pairing code using cryptographically secure randomness.
fn generate_code() -> String {
    // UUID v4 uses getrandom (backed by /dev/urandom on Linux, BCryptGenRandom
    // on Windows) — a CSPRNG. We extract 4 bytes from it for a uniform random
    // number in [0, 1_000_000).
    //
    // Rejection sampling eliminates modulo bias: values above the largest
    // multiple of 1_000_000 that fits in u32 are discarded and re-drawn.
    // The rejection probability is ~0.02%, so this loop almost always exits
    // on the first iteration.
    const UPPER_BOUND: u32 = 1_000_000;
    const REJECT_THRESHOLD: u32 = (u32::MAX / UPPER_BOUND) * UPPER_BOUND;

    loop {
        let uuid = uuid::Uuid::new_v4();
        let bytes = uuid.as_bytes();
        let raw = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);

        if raw < REJECT_THRESHOLD {
            return format!("{:06}", raw % UPPER_BOUND);
        }
    }
}

/// Generate a cryptographically-adequate bearer token with 256-bit entropy.
///
/// Uses `rand::rng()` which is backed by the OS CSPRNG
/// (/dev/urandom on Linux, BCryptGenRandom on Windows, SecRandomCopyBytes
/// on macOS). The 32 random bytes (256 bits) are hex-encoded for a
/// 64-character token, providing 256 bits of entropy.
fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    format!("zc_{}", hex::encode(bytes))
}

/// SHA-256 hash a bearer token for storage. Returns lowercase hex.
fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

/// Check if a stored value looks like a SHA-256 hash (64 hex chars)
/// rather than a plaintext token.
fn is_token_hash(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// Constant-time string comparison to prevent timing attacks.
///
/// Does not short-circuit on length mismatch — always iterates over the
/// longer input to avoid leaking length information via timing.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();

    // Track length mismatch as a usize (non-zero = different lengths)
    let len_diff = a.len() ^ b.len();

    // XOR each byte, padding the shorter input with zeros.
    // Iterates over max(a.len(), b.len()) to avoid timing differences.
    let max_len = a.len().max(b.len());
    let mut byte_diff = 0u8;
    for i in 0..max_len {
        let x = *a.get(i).unwrap_or(&0);
        let y = *b.get(i).unwrap_or(&0);
        byte_diff |= x ^ y;
    }
    (len_diff == 0) & (byte_diff == 0)
}

/// Check if a host string represents a non-localhost bind address.
pub fn is_public_bind(host: &str) -> bool {
    !matches!(
        host,
        "127.0.0.1" | "localhost" | "::1" | "[::1]" | "0:0:0:0:0:0:0:1"
    )
}

#[cfg(test)]
#[path = "pairing_tests.rs"]
mod tests;
