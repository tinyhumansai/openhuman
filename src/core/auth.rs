//! Per-process RPC bearer-token authentication.
//!
//! At server startup, [`init_rpc_token`] generates a 256-bit
//! cryptographically-random token, writes it to
//! `{workspace_dir}/core.token` (owner-read-only on Unix), and stores it in a
//! process-global [`OnceLock`].  The Tauri shell reads that file and includes
//! the token in every request as `Authorization: Bearer <token>`.
//!
//! Endpoints exempt from auth (checked by [`rpc_auth_middleware`]):
//! - `GET /`              — public info page
//! - `GET /health`        — liveness probe
//! - `GET /auth/telegram` — external browser callback (carries its own token)
//! - `GET /schema`        — read-only schema discovery
//! - `GET /events`        — SSE stream; browser `EventSource` cannot set headers
//! - `GET /events/webhooks` — webhook SSE; same browser constraint
//! - `GET /ws/dictation`  — WebSocket upgrade; browser WS API cannot set headers
//! - `OPTIONS *`          — CORS preflight (handled by outer CORS middleware)
//!
//! Only `POST /rpc` carries executable commands and requires the bearer token.

use std::io::Write as _;
use std::path::Path;
use std::sync::OnceLock;

use axum::http::{header, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

static RPC_TOKEN: OnceLock<String> = OnceLock::new();

/// Paths that bypass bearer-token authentication.
///
/// Only `/rpc` carries executable commands and must be protected.  All other
/// routes are read-only, streaming, or WebSocket upgrades whose clients
/// (browser `EventSource`, browser `WebSocket`) cannot set `Authorization`
/// headers via standard APIs.
const PUBLIC_PATHS: &[&str] = &[
    "/",
    "/health",
    "/auth/telegram",
    "/schema",
    "/events",
    "/events/webhooks",
    "/ws/dictation",
];

/// The environment variable the Tauri shell sets before spawning the core.
///
/// When this variable is present the core uses its value as the RPC token
/// (no file I/O needed).  When absent (standalone `openhuman core run`) the
/// core generates a token and writes it to `{workspace_dir}/core.token` so
/// CLI clients can authenticate.
pub const CORE_TOKEN_ENV_VAR: &str = "OPENHUMAN_CORE_TOKEN";

/// Initialize the per-process RPC token.
///
/// **Preferred path — Tauri-spawned core**: reads the token from the
/// `OPENHUMAN_CORE_TOKEN` environment variable set by the Tauri shell.  No
/// file is written; the token is always available the instant the process
/// starts.
///
/// **Fallback — standalone CLI**: generates a fresh 256-bit token, writes it
/// to `{workspace_dir}/core.token` (owner-read-only on Unix) for external
/// callers, and stores it in the process global.
///
/// # Errors
///
/// Returns an error only in the fallback path, if the token file cannot be
/// written.
pub fn init_rpc_token(workspace_dir: &Path) -> anyhow::Result<()> {
    // Idempotency guard: if the token is already set, do nothing.  A second
    // call must never write a new token to disk while the process still
    // validates the original in-memory value — that would cause clients
    // reading core.token to start getting 401s immediately.
    if RPC_TOKEN.get().is_some() {
        log::debug!("[auth] init_rpc_token: already initialized, skipping");
        return Ok(());
    }

    // Fast path: token pre-seeded by the Tauri shell via env var.
    if let Ok(env_token) = std::env::var(CORE_TOKEN_ENV_VAR) {
        let env_token = env_token.trim().to_string();
        if !env_token.is_empty() {
            let _ = RPC_TOKEN.set(env_token);
            log::info!("[auth] core RPC token loaded from environment (Tauri-managed)");
            return Ok(());
        }
    }

    // Fallback: standalone CLI — generate and write to file.
    let token = generate_token();
    let token_path = workspace_dir.join("core.token");
    write_token_file(&token_path, &token)?;
    let _ = RPC_TOKEN.set(token);
    log::info!(
        "[auth] core RPC token generated and written to {}",
        token_path.display()
    );
    Ok(())
}

/// Returns the active RPC token, if initialized.
pub fn get_rpc_token() -> Option<&'static str> {
    RPC_TOKEN.get().map(String::as_str)
}

/// Axum middleware: enforce `Authorization: Bearer <token>` on all protected
/// endpoints.
///
/// Public paths (see [`PUBLIC_PATHS`]) and CORS preflight `OPTIONS` requests
/// bypass this check.  All other requests must carry the exact bearer token
/// that was written to `core.token` at startup.
pub async fn rpc_auth_middleware(req: axum::extract::Request, next: Next) -> Response {
    let path = req.uri().path().to_string();

    // CORS preflight and public utility paths bypass auth.
    if req.method() == Method::OPTIONS || PUBLIC_PATHS.contains(&path.as_str()) {
        return next.run(req).await;
    }

    let Some(expected) = get_rpc_token() else {
        // Shouldn't happen in production — token is always initialized before
        // the router starts serving. Deny to be safe.
        log::error!("[auth] RPC token not initialized — denying request to {path}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "ok": false,
                "error": "server_error",
                "message": "Auth subsystem not initialized"
            })),
        )
            .into_response();
    };

    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if bearer
        .strip_prefix("Bearer ")
        .is_some_and(|token| token == expected)
    {
        log::trace!("[auth] authorized request to {path}");
        next.run(req).await
    } else {
        log::warn!("[auth] unauthorized request to {path} — missing or wrong bearer token");
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "ok": false,
                "error": "unauthorized",
                "message": "Missing or invalid Authorization header. Supply 'Authorization: Bearer <token>'."
            })),
        )
            .into_response()
    }
}

/// Generate a 256-bit cryptographically-random token as a lowercase hex string.
///
/// Uses `rand::rng()` (thread-local, OS-seeded CSPRNG) introduced in rand 0.9.
fn generate_token() -> String {
    use rand::RngCore as _;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Write `token` to `path` with owner-only read+write permissions on Unix.
fn write_token_file(path: &Path, token: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
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
mod tests {
    use super::*;

    #[test]
    fn generate_token_produces_64_hex_chars() {
        let t = generate_token();
        assert_eq!(t.len(), 64, "256 bits → 64 hex chars");
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()), "must be hex");
    }

    #[test]
    fn generate_token_is_not_constant() {
        assert_ne!(generate_token(), generate_token());
    }

    #[test]
    fn write_and_read_token_roundtrips() {
        let tmp = std::env::temp_dir().join(format!("core-auth-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("core.token");
        let token = "cafebabe1234567890abcdef0123456789abcdef0123456789abcdef01234567";
        write_token_file(&path, token).unwrap();
        let back = std::fs::read_to_string(&path).unwrap();
        assert_eq!(back, token);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[cfg(unix)]
    #[test]
    fn token_file_has_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = std::env::temp_dir().join(format!("core-auth-perms-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("core.token");
        write_token_file(&path, "abc").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "token file must be 0o600");
        std::fs::remove_dir_all(&tmp).ok();
    }
}
