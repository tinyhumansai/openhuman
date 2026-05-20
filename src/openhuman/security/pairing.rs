// First-connect authentication for channels (e.g. Telegram) that support operator pairing.
//
// A one-time pairing code can be shown to the operator; successful pairing issues
// a bearer token. Tokens can be persisted in config so restarts don't require
// re-pairing.

use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

/// Environment variable for the core JSON-RPC bearer token (see `crate::core::auth`).
pub const CORE_TOKEN_ENV_VAR: &str = "OPENHUMAN_CORE_TOKEN";

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
    pub fn new(require_pairing: bool, existing_tokens: &[String]) -> (Self, Option<String>) {
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
        let guard = Self {
            require_pairing,
            pairing_code: Arc::new(Mutex::new(code.clone())),
            paired_tokens: Arc::new(Mutex::new(tokens)),
            failed_attempts: Arc::new(Mutex::new((0, None))),
        };
        (guard, code)
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
    ///
    /// Always fails closed on empty/whitespace tokens. When pairing is not required,
    /// configured tokens are still honored if present; with no tokens configured,
    /// every request is rejected.
    pub fn is_authenticated(&self, token: &str) -> bool {
        if token.trim().is_empty() {
            log::debug!("[openhuman:pairing] is_authenticated: rejected empty bearer token");
            return false;
        }

        let tokens = self.paired_tokens.lock();
        if tokens.is_empty() {
            log::debug!(
                "[openhuman:pairing] is_authenticated: no paired tokens configured (require_pairing={})",
                self.require_pairing
            );
            return false;
        }

        let hashed = hash_token(token);
        let ok = tokens.contains(&hashed);
        if !ok {
            log::debug!("[openhuman:pairing] is_authenticated: bearer token not in paired set");
        }
        ok
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
    use rand::RngExt as _;
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
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
        host.trim(),
        "127.0.0.1" | "localhost" | "::1" | "[::1]" | "0:0:0:0:0:0:0:1"
    )
}

/// Error while resolving or persisting a core RPC token for a bind address.
#[derive(Debug, thiserror::Error)]
pub enum CoreBindTokenError {
    #[error(
        "{CORE_TOKEN_ENV_VAR} must not be empty when binding on a non-loopback address ({host})"
    )]
    EmptyEnvToken { host: String },
    #[error("failed to persist core RPC token at {path}: {source}")]
    Persist {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Ensure a non-empty core RPC bearer token exists before binding on `host`.
///
/// **Loopback** (`127.0.0.1`, `localhost`, `::1`, …): returns `Ok(None)` when
/// `env_token` is unset/empty so local dev can rely on other startup paths.
///
/// **Non-loopback** (`0.0.0.0`, LAN IPs, …): returns a usable token — either the
/// trimmed `env_token` or a freshly generated 256-bit value written to
/// `{workspace_dir}/core.token` (owner-only on Unix), matching the standalone CLI
/// path in `crate::core::auth::init_rpc_token`.
pub fn ensure_core_rpc_token_for_bind(
    host: &str,
    workspace_dir: &Path,
    env_token: Option<&str>,
) -> Result<Option<String>, CoreBindTokenError> {
    let host = host.trim();
    if let Some(raw) = env_token {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            log::info!(
                "[openhuman:pairing] core RPC token supplied via {CORE_TOKEN_ENV_VAR} for bind host={host}"
            );
            return Ok(Some(trimmed.to_string()));
        }
        if is_public_bind(host) {
            log::error!(
                "[openhuman:pairing] {CORE_TOKEN_ENV_VAR} is set but empty on public bind host={host}"
            );
            return Err(CoreBindTokenError::EmptyEnvToken {
                host: host.to_string(),
            });
        }
    }

    if !is_public_bind(host) {
        log::debug!(
            "[openhuman:pairing] loopback bind host={host}: no {CORE_TOKEN_ENV_VAR} configured"
        );
        return Ok(None);
    }

    let token = generate_core_rpc_token();
    let token_path = workspace_dir.join("core.token");
    write_core_token_file(&token_path, &token).map_err(|source| CoreBindTokenError::Persist {
        path: token_path.display().to_string(),
        source,
    })?;
    log::warn!(
        "[openhuman:pairing] Public bind on {host} without {CORE_TOKEN_ENV_VAR}: \
         generated token at {} — set {CORE_TOKEN_ENV_VAR} explicitly for stable deployments",
        token_path.display()
    );
    Ok(Some(token))
}

/// Generate a 256-bit core RPC bearer token (lowercase hex, no `zc_` prefix).
fn generate_core_rpc_token() -> String {
    use rand::RngExt as _;
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    hex::encode(bytes)
}

/// Write `token` to `path` with owner-only permissions on Unix (`0o600`).
fn write_core_token_file(path: &Path, token: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(unix)]
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(token.as_bytes())?;
    }

    #[cfg(not(unix))]
    {
        std::fs::write(path, token)?;
    }

    Ok(())
}

#[cfg(test)]
#[path = "pairing_tests.rs"]
mod tests;
