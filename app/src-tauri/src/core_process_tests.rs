use super::{current_rpc_token, default_core_port, generate_rpc_token, CoreProcessHandle};
use std::sync::{Mutex, MutexGuard, OnceLock};

fn env_lock() -> MutexGuard<'static, ()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock poisoned")
}

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

#[test]
fn default_core_port_env_and_fallback() {
    let _env_lock = env_lock();
    let _unset = EnvGuard::unset("OPENHUMAN_CORE_PORT");
    assert_eq!(default_core_port(), 7788);

    let _set = EnvGuard::set("OPENHUMAN_CORE_PORT", "8899");
    assert_eq!(default_core_port(), 8899);
}

#[test]
fn core_process_handle_new_creates_instance() {
    let handle = CoreProcessHandle::new(9999);
    assert_eq!(handle.port(), 9999);
    assert_eq!(handle.rpc_url(), "http://127.0.0.1:9999/rpc");
}

#[test]
fn ensure_running_returns_ok_when_rpc_port_already_open() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let result = rt.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let port = listener.local_addr().expect("local addr").port();
        let handle = CoreProcessHandle::new(port);
        handle.ensure_running().await
    });
    assert!(
        result.is_ok(),
        "ensure_running should fast-path: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Token generation tests
// ---------------------------------------------------------------------------

/// `generate_rpc_token` must produce a 64-character lowercase hex string
/// (32 bytes × 2 hex digits = 64 chars), matching the format expected by the
/// core's auth middleware.
#[test]
fn generate_rpc_token_produces_64_hex_chars() {
    let token = generate_rpc_token();
    assert_eq!(
        token.len(),
        64,
        "256-bit token → 64 hex chars, got {token:?}"
    );
    assert!(
        token.chars().all(|c| c.is_ascii_hexdigit()),
        "token must be hex, got {token:?}"
    );
    assert!(
        token.chars().all(|c| !c.is_uppercase()),
        "token must be lowercase hex, got {token:?}"
    );
}

/// Each call generates a different token (CSPRNG — not a constant).
#[test]
fn generate_rpc_token_is_not_constant() {
    assert_ne!(
        generate_rpc_token(),
        generate_rpc_token(),
        "two consecutive tokens must differ"
    );
}

/// `CoreProcessHandle::new` must produce a non-empty, correctly-formatted
/// bearer token immediately — no file I/O or timing dependency.
#[test]
fn core_process_handle_new_token_is_valid() {
    let handle = CoreProcessHandle::new(19001);
    let token = handle.rpc_token();
    assert_eq!(token.len(), 64, "handle token must be 64 hex chars");
    assert!(
        token.chars().all(|c| c.is_ascii_hexdigit()),
        "handle token must be hex"
    );
}

/// `CoreProcessHandle::new()` must NOT publish the token to the global
/// `CURRENT_RPC_TOKEN`. The global is set only after `ensure_running()`
/// successfully spawns the embedded server with `OPENHUMAN_CORE_TOKEN` in
/// scope. Advertising the token before spawn would 401 against any process
/// already listening on the port that never received this token.
#[test]
fn new_does_not_publish_global_token() {
    let before = current_rpc_token();
    let handle = CoreProcessHandle::new(19002);
    let after = current_rpc_token();

    assert_ne!(
        after.as_deref(),
        Some(handle.rpc_token()),
        "new() must not publish its token to CURRENT_RPC_TOKEN before ensure_running() spawns"
    );
    assert_eq!(
        before, after,
        "new() must leave CURRENT_RPC_TOKEN unchanged"
    );
}

/// Two handles constructed sequentially must each have a unique token.
#[test]
fn each_handle_has_unique_token() {
    let h1 = CoreProcessHandle::new(19003);
    let h2 = CoreProcessHandle::new(19004);

    assert_ne!(
        h1.rpc_token(),
        h2.rpc_token(),
        "each handle must have a unique token"
    );
}
