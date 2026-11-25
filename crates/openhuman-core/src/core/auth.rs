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
//! transports (the HTTP auth middleware, Socket.IO, SSE query-token fallback,
//! the approval-gate session id) read via [`get_rpc_token`].
//!
//! Which HTTP routes require the bearer is the server's policy, in
//! `openhuman_rpc::server::auth`.

use std::path::Path;
use std::sync::OnceLock;

// The axum RPC-auth middleware + the `/v1` external-inference bearer helpers
// are exclusive to the `http-server` feature (#5048). The always-compiled token
// primitives (`verify_bearer_token`, `init_rpc_token`, `get_rpc_token`,
// `init_rpc_token_with_value`, `bearer_matches`) stay ungated — non-HTTP
// transports and `CoreBuilder` call them in every build. `Config`/`AuthService`/
// the provider-id import are consumed only by the gated `/v1` helpers.

static RPC_TOKEN: OnceLock<String> = OnceLock::new();

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

/// Single source of truth for token comparison.
///
/// Use constant-time equality so callers that validate attacker-controlled
/// bearer strings do not leak partial-match timing through HTTP, SSE, Socket.IO,
/// or future transports that share this helper.
pub fn bearer_matches(supplied: &str, expected: &str) -> bool {
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
///
/// Published through a fresh 0600 inode renamed over the old file (the shared
/// keyring writer): chmod on an existing file neither revokes descriptors
/// already open on it nor survives a failed rewrite, and the old token stays
/// intact until the rename succeeds.
fn write_token_file(path: &Path, token: &str) -> anyhow::Result<()> {
    crate::security::secret_storage::file_store::write_atomic(path, token.as_bytes())?;
    Ok(())
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
