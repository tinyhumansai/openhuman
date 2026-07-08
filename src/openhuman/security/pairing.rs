// First-connect authentication for channels (e.g. Telegram) that support operator pairing.
//
// A one-time pairing code can be shown to the operator; successful pairing issues
// a bearer token. Tokens can be persisted in config so restarts don't require
// re-pairing.

use std::io::Write as _;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;

/// `PairingGuard`, `constant_time_eq`, and the pairing lockout constants now
/// live in tinychannels (portable, self-contained); re-export them so existing
/// callers (channels, `core::auth`, tests) keep their paths.
pub use tinychannels::security::{
    constant_time_eq, generate_code, generate_token, hash_token, is_token_hash, PairingGuard,
    MAX_PAIR_ATTEMPTS, PAIR_LOCKOUT_SECS,
};

/// Environment variable for the core JSON-RPC bearer token (see `crate::core::auth`).
pub const CORE_TOKEN_ENV_VAR: &str = "OPENHUMAN_CORE_TOKEN";

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
