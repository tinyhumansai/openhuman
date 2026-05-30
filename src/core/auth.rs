//! Per-process RPC bearer-token authentication.
//!
//! Three initialization paths feed the process-global [`OnceLock`] that holds
//! the active bearer token:
//!
//! 1. **In-memory handoff (preferred for the in-process core)** —
//!    [`init_rpc_token_with_value`] sets the token directly from a value the
//!    Tauri shell already holds in `CoreProcessHandle.rpc_token`. No env var
//!    is read or set; the token never crosses a process-global env surface.
//!    This is the path the Tauri host uses now that the core runs in-process
//!    (PR #1061) — same-process handoff makes the env crossing unnecessary,
//!    and avoiding it keeps the token off `/proc/<pid>/environ` (Linux) and
//!    out of `sysctl KERN_PROCARGS2` / `ps eww -p <pid>` (macOS) where any
//!    same-UID process could read it without entitlement.
//! 2. **Env-as-config fallback** — when no in-memory token is supplied,
//!    [`init_rpc_token`] reads `OPENHUMAN_CORE_TOKEN` from the environment.
//!    This is the legitimate operator-supplied transport for Docker / cloud /
//!    VPS deployments where the bearer must come from `fly secrets set …`,
//!    `docker run -e …`, or a systemd unit file — there is no live shell
//!    handing it to the binary in-memory.
//! 3. **Standalone CLI fallback** — when neither path supplies a token, the
//!    core generates a fresh 256-bit token and writes it to
//!    `{workspace_dir}/core.token` (owner-read-only on Unix) so external CLI
//!    clients can authenticate.
//!
//! Once set, the in-memory `OnceLock` is the single source of truth — all
//! transports ([`rpc_auth_middleware`], Socket.IO, SSE query-token fallback,
//! the approval-gate session id) read via [`get_rpc_token`].
//!
//! Endpoints exempt from auth (checked by [`rpc_auth_middleware`]):
//! - `GET /`              — public info page
//! - `GET /health`        — liveness probe
//! - `GET /auth`          — desktop login callback fallback; consumes only
//!                          one-time login tokens, never raw session JWTs
//! - `GET /auth/telegram` — external browser callback (carries its own token)
//! - `GET /schema`        — read-only schema discovery
//! - `GET /events`        — SSE stream; browser `EventSource` cannot set headers
//! - `GET /ws/dictation`  — WebSocket upgrade; browser WS API cannot set headers
//! - `OPTIONS *`          — CORS preflight (handled by outer CORS middleware)
//!
//! Endpoints that accept the bearer either via header **or** `?token=…` query
//! param (see [`QUERY_TOKEN_PATHS`]):
//! - `GET /events/webhooks` — webhook SSE; browser `EventSource` cannot set
//!   headers, so the FE forwards the bearer as a query param. Validated
//!   against the same in-process RPC token — no separate secret.
//!
//! Executable surfaces:
//! - `POST /rpc` requires the per-launch core bearer token.
//! - `GET /v1/models` and `POST /v1/chat/completions` accept either that
//!   internal bearer or a stable user-managed external API key stored under
//!   `openhuman::inference::http::EXTERNAL_OPENAI_COMPAT_PROVIDER`.

use std::io::Write as _;
use std::path::Path;
use std::sync::OnceLock;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;

use axum::http::{header, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::credentials::AuthService;
use crate::openhuman::inference::http::EXTERNAL_OPENAI_COMPAT_PROVIDER;

static RPC_TOKEN: OnceLock<String> = OnceLock::new();

/// Paths that bypass bearer-token authentication.
///
/// `/rpc` and `/v1/*` carry executable surfaces and must be protected. All
/// other routes are read-only, streaming, or WebSocket upgrades whose clients
/// (browser `EventSource`, browser `WebSocket`) cannot set `Authorization`
/// headers via standard APIs.
const PUBLIC_PATHS: &[&str] = &[
    "/",
    "/health",
    "/auth",
    "/auth/telegram",
    "/schema",
    "/events",
    "/ws/dictation",
];

/// Paths that may authenticate via `?token=…` in the URL when no
/// `Authorization` header is present.
///
/// Browser `EventSource` cannot attach custom headers, so an SSE route that
/// returns sensitive data (webhook deliveries, registration changes) is
/// otherwise indistinguishable from a public endpoint — any local process on
/// `127.0.0.1` can subscribe. Allowing the bearer in the query string lets
/// the FE attach it explicitly while keeping a single token of truth
/// (validated by [`bearer_matches`] against the same in-process RPC token).
///
/// Add new entries here only for SSE / WebSocket routes whose clients cannot
/// send headers and that carry per-user data. The follow-up approvals stream
/// (#1339) is the next planned addition.
const QUERY_TOKEN_PATHS: &[&str] = &["/events/webhooks"];

/// Operator-supplied environment variable that carries the RPC bearer in
/// non-desktop deployments.
///
/// **The Tauri desktop shell does NOT set this variable.** Since PR #1061
/// the core runs in-process inside the Tauri host, and the shell hands the
/// per-launch bearer to the embedded server via an internal in-memory handle
/// (see [`init_rpc_token_with_value`]). The desktop boot flow never crosses
/// a process-global env surface.
///
/// `OPENHUMAN_CORE_TOKEN` remains the canonical configuration surface for
/// **standalone CLI / Docker / cloud** deployments only — where the bearer
/// must come from `fly secrets set …`, `docker run -e …`, a systemd unit
/// file, or a developer running `openhuman-core serve` from a shell with the
/// env var pre-set. In those shapes there is no live host process to hand
/// the token over in-memory, so env-as-config is the appropriate transport.
///
/// When this variable is present [`init_rpc_token`] uses its value (no file
/// I/O). When absent and no in-memory token was seeded, `init_rpc_token`
/// generates a fresh token and writes it to `{workspace_dir}/core.token` so
/// CLI clients can authenticate.
pub const CORE_TOKEN_ENV_VAR: &str = "OPENHUMAN_CORE_TOKEN";

/// Initialize the per-process RPC token from env-or-file (non-desktop path).
///
/// **Not the desktop path.** The Tauri shell passes the per-launch bearer
/// to the embedded server via the internal in-memory handle (see
/// [`init_rpc_token_with_value`]); it does **not** set
/// `OPENHUMAN_CORE_TOKEN`. This function is the bootstrap path for
/// standalone CLI / Docker / cloud deployments.
///
/// **Env-as-config (preferred for non-desktop)**: when
/// `OPENHUMAN_CORE_TOKEN` is set in the process environment (typically by
/// the container runtime, secrets manager, or systemd unit file), the core
/// uses its value as the RPC token. No file is written; the token is
/// available the instant the process starts.
///
/// **Standalone CLI fallback**: when no env var is supplied, the core
/// generates a fresh 256-bit token, writes it to `{workspace_dir}/core.token`
/// (owner-read-only on Unix) for external callers, and stores it in the
/// process global.
///
/// # Errors
///
/// Returns an error only in the standalone fallback path, if the token file
/// cannot be written.
pub fn init_rpc_token(workspace_dir: &Path) -> anyhow::Result<()> {
    // Idempotency guard: if the token is already set, do nothing.  A second
    // call must never write a new token to disk while the process still
    // validates the original in-memory value — that would cause clients
    // reading core.token to start getting 401s immediately.
    if RPC_TOKEN.get().is_some() {
        log::debug!("[auth] init_rpc_token: already initialized, skipping");
        return Ok(());
    }

    // Env-as-config path: bearer supplied by the operator via
    // OPENHUMAN_CORE_TOKEN. Used by Docker / cloud / systemd / a developer
    // running `openhuman-core serve` from a pre-configured shell. Desktop
    // (Tauri) does NOT set this variable — it uses `init_rpc_token_with_value`
    // for an in-memory handoff instead.
    if let Ok(env_token) = std::env::var(CORE_TOKEN_ENV_VAR) {
        let env_token = env_token.trim().to_string();
        if !env_token.is_empty() {
            let _ = RPC_TOKEN.set(env_token);
            log::info!("[auth] core RPC token loaded from environment (operator-supplied)");
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

/// Seed the per-process RPC token directly from a caller-supplied value.
///
/// **In-memory handoff path** — used by the Tauri shell to inject the bearer
/// the host generated in `CoreProcessHandle::new()` into the in-process core
/// without round-tripping through `OPENHUMAN_CORE_TOKEN` in the process
/// environment. The token never lands on a process-global env surface, which
/// keeps it off `/proc/<pid>/environ` (Linux) and out of `sysctl
/// KERN_PROCARGS2` / `ps eww -p <pid>` (macOS) where any same-UID process
/// could otherwise read it without entitlement.
///
/// Idempotent: a second call is a no-op (matches [`init_rpc_token`] — flipping
/// the in-memory bearer mid-life would 401 every in-flight client).
///
/// # Errors
///
/// Returns an error only if `token` is empty after trimming. A non-empty
/// token is accepted as-is — callers are expected to have generated a
/// CSPRNG hex string (see `CoreProcessHandle::generate_rpc_token`).
pub fn init_rpc_token_with_value(token: &str) -> anyhow::Result<()> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        anyhow::bail!("init_rpc_token_with_value: supplied token is empty");
    }
    if RPC_TOKEN.get().is_some() {
        log::debug!("[auth] init_rpc_token_with_value: already initialized, skipping");
        return Ok(());
    }
    let _ = RPC_TOKEN.set(trimmed.to_string());
    log::info!("[auth] core RPC token loaded via in-memory handoff (no env crossing)");
    Ok(())
}

/// Returns the active RPC token, if initialized.
pub fn get_rpc_token() -> Option<&'static str> {
    RPC_TOKEN.get().map(String::as_str)
}

/// Validate a supplied bearer token against the active per-process RPC token.
///
/// Returns `true` only when the token subsystem is initialised and the
/// supplied token is non-empty and matches the in-memory expected value.
///
/// This is the single entry point that non-HTTP transports (Socket.IO event
/// handlers, SSE bind-token issuance, future WebSocket surfaces) should call
/// before letting attacker-controlled input reach executable code. Keeping
/// the comparison in one helper means every transport gets the same
/// constant-time equality semantics.
pub fn verify_bearer_token(supplied: &str) -> bool {
    let Some(expected) = get_rpc_token() else {
        return false;
    };
    bearer_matches(supplied, expected)
}

/// Axum middleware: enforce `Authorization: Bearer <token>` on all protected
/// endpoints.
///
/// Public paths (see [`PUBLIC_PATHS`]) and CORS preflight `OPTIONS` requests
/// bypass this check. `/rpc` requires the exact per-launch bearer token that
/// was written to `core.token` at startup; `/v1/*` additionally accepts a
/// stable user-managed external API key.
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

    let header_token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    if bearer_matches(header_token, expected) {
        log::trace!("[auth] authorized request to {path} (header)");
        return next.run(req).await;
    }

    if is_external_inference_path(&path) && verify_external_inference_bearer(header_token).await {
        log::trace!("[auth] authorized request to {path} (external inference bearer)");
        return next.run(req).await;
    }

    // Header path failed — fall back to `?token=…` for SSE/WS routes whose
    // browser clients cannot set headers. The query token is validated
    // against the same in-process RPC bearer (single source of truth), so
    // this is not a separate credential — only a transport workaround.
    if QUERY_TOKEN_PATHS.contains(&path.as_str()) {
        if let Some(query_token) = extract_query_token(req.uri().query()) {
            if bearer_matches(&query_token, expected) {
                log::trace!("[auth] authorized request to {path} (query token)");
                return next.run(req).await;
            }
        }
    }

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

/// Single source of truth for token comparison.
///
/// Use constant-time equality so callers that validate attacker-controlled
/// bearer strings do not leak partial-match timing through HTTP, SSE, Socket.IO,
/// or future transports that share this helper.
fn bearer_matches(supplied: &str, expected: &str) -> bool {
    !supplied.is_empty() && constant_time_eq(supplied, expected)
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let len_diff = a.len() ^ b.len();
    let max_len = a.len().max(b.len());
    let mut byte_diff = 0u8;

    for i in 0..max_len {
        let left = *a.get(i).unwrap_or(&0);
        let right = *b.get(i).unwrap_or(&0);
        byte_diff |= left ^ right;
    }

    (len_diff == 0) & (byte_diff == 0)
}

fn is_external_inference_path(path: &str) -> bool {
    path == "/v1" || path.starts_with("/v1/")
}

fn verify_external_inference_bearer_for_config(config: &Config, supplied: &str) -> bool {
    if supplied.trim().is_empty() {
        return false;
    }

    let auth = AuthService::from_config(config);
    match auth.get_provider_bearer_token(EXTERNAL_OPENAI_COMPAT_PROVIDER, None) {
        Ok(Some(expected)) => bearer_matches(supplied, expected.trim()),
        Ok(None) => false,
        Err(err) => {
            log::warn!("[auth] failed to read external inference bearer: {err}");
            false
        }
    }
}

async fn verify_external_inference_bearer(supplied: &str) -> bool {
    if supplied.trim().is_empty() {
        return false;
    }

    let config = match Config::load_or_init().await {
        Ok(config) => config,
        Err(err) => {
            log::warn!("[auth] failed to load config for external inference bearer: {err}");
            return false;
        }
    };

    verify_external_inference_bearer_for_config(&config, supplied)
}

/// Pull the first `token` query parameter out of a URL query string.
///
/// Returns `None` when the query is absent, the key is missing, or the
/// value is empty after trimming. URL decoding is delegated to
/// [`url::form_urlencoded`] so percent-encoded tokens decode the same way
/// they were encoded by the FE via `encodeURIComponent`.
fn extract_query_token(query: Option<&str>) -> Option<String> {
    let query = query?;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key == "token" {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// Generate a 256-bit cryptographically-random token as a lowercase hex string.
///
/// Uses `rand::rng()` (thread-local, OS-seeded CSPRNG) introduced in rand 0.9.
fn generate_token() -> String {
    use rand::RngExt as _;
    log::trace!("[auth] generate_token: start (32 bytes)");
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    let token = hex::encode(bytes);
    log::trace!("[auth] generate_token: complete (64 hex chars)");
    token
}

/// Write `token` to `path` with owner-only read+write permissions on Unix.
fn write_token_file(path: &Path, token: &str) -> anyhow::Result<()> {
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

    #[test]
    fn bearer_matches_rejects_empty_supplied() {
        let expected = "cafebabe";
        assert!(!bearer_matches("", expected));
    }

    #[test]
    fn bearer_matches_rejects_mismatch() {
        assert!(!bearer_matches("deadbeef", "cafebabe"));
    }

    #[test]
    fn bearer_matches_rejects_prefix_match() {
        assert!(!bearer_matches("cafeba", "cafebabe"));
    }

    #[test]
    fn bearer_matches_accepts_exact() {
        assert!(bearer_matches("cafebabe", "cafebabe"));
    }

    #[test]
    fn verify_bearer_token_returns_false_when_token_uninitialized() {
        // RPC_TOKEN is a process-global OnceLock; on a fresh test binary it
        // may already be set by another test that ran first, so we cannot
        // assert the uninitialized branch here without process isolation.
        // We can however confirm that an empty supplied value is always
        // rejected, which exercises the second-leg invariant.
        assert!(!verify_bearer_token(""));
    }

    #[test]
    fn init_rpc_token_with_value_rejects_empty() {
        // Trimmed-empty values must error rather than seed an empty bearer.
        assert!(init_rpc_token_with_value("").is_err());
        assert!(init_rpc_token_with_value("   ").is_err());
    }

    /// `init_rpc_token_with_value` populates the same `RPC_TOKEN` OnceLock
    /// that `get_rpc_token` reads — i.e. the in-memory handoff path produces
    /// the bearer everyone else (HTTP middleware, Socket.IO verifier,
    /// approval-gate session_id) reads from. We can't deterministically
    /// assert the *value* set here (the OnceLock may already be seeded by a
    /// sibling test that ran first in the same binary), but we can assert
    /// the OnceLock is initialised after this call returns Ok, and that the
    /// helper is idempotent.
    #[test]
    fn init_rpc_token_with_value_seeds_and_is_idempotent() {
        // First call: either we seed, or a sibling test already did. Either
        // way the helper must return Ok and leave `get_rpc_token` populated.
        let token = "cafebabe1234567890abcdef0123456789abcdef0123456789abcdef01234567";
        init_rpc_token_with_value(token).expect("seed succeeds");
        assert!(
            get_rpc_token().is_some(),
            "after init_rpc_token_with_value, get_rpc_token must return Some"
        );
        // Second call is a no-op (matching init_rpc_token semantics) — must
        // not error, must not flip the in-memory value.
        let before = get_rpc_token().map(str::to_string);
        init_rpc_token_with_value("a-different-value-that-must-be-ignored")
            .expect("idempotent re-init succeeds");
        let after = get_rpc_token().map(str::to_string);
        assert_eq!(
            before, after,
            "second init_rpc_token_with_value must not flip the in-memory bearer"
        );
    }

    #[test]
    fn extract_query_token_returns_none_on_missing_query() {
        assert_eq!(extract_query_token(None), None);
    }

    #[test]
    fn extract_query_token_returns_none_when_key_absent() {
        assert_eq!(extract_query_token(Some("other=1&foo=bar")), None);
    }

    #[test]
    fn extract_query_token_returns_none_on_empty_value() {
        assert_eq!(extract_query_token(Some("token=")), None);
        assert_eq!(extract_query_token(Some("token=%20%20")), None);
    }

    #[test]
    fn extract_query_token_returns_first_value_on_duplicate_keys() {
        // Last-wins vs first-wins is a question the FE never hits; pin
        // first-wins so any future ambiguity is documented.
        assert_eq!(
            extract_query_token(Some("token=alpha&token=beta")),
            Some("alpha".to_string())
        );
    }

    #[test]
    fn extract_query_token_url_decodes_value() {
        // `encodeURIComponent` on the FE may percent-encode a hex token
        // accidentally (it shouldn't, but defensive); confirm round-trip.
        assert_eq!(
            extract_query_token(Some("token=cafe%2Dbabe")),
            Some("cafe-babe".to_string())
        );
    }

    #[test]
    fn public_paths_include_desktop_auth_callback() {
        assert!(PUBLIC_PATHS.contains(&"/auth"));
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

    #[test]
    fn is_external_inference_path_matches_only_v1_routes() {
        assert!(is_external_inference_path("/v1"));
        assert!(is_external_inference_path("/v1/models"));
        assert!(is_external_inference_path("/v1/chat/completions"));
        assert!(!is_external_inference_path("/rpc"));
        assert!(!is_external_inference_path("/v10/models"));
    }

    #[test]
    fn verify_external_inference_bearer_for_config_accepts_stored_key() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");

        let auth = AuthService::from_config(&config);
        auth.store_provider_token(
            EXTERNAL_OPENAI_COMPAT_PROVIDER,
            "default",
            "external-test-key",
            std::collections::HashMap::new(),
            true,
        )
        .unwrap();

        assert!(verify_external_inference_bearer_for_config(
            &config,
            "external-test-key"
        ));
        assert!(!verify_external_inference_bearer_for_config(
            &config,
            "wrong-key"
        ));
    }
}
