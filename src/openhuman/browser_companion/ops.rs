//! Business logic for the Browser Companion domain: owns the lifecycle of the
//! TinyFlows `CompanionServer` (loopback WebSocket relay to the Chrome
//! extension) and its pairing secret.
//!
//! Increment 1 scope: lifecycle + pairing only. No RPC controllers and no
//! flows wiring yet — those land in later stages. See
//! `src/openhuman/browser_companion/mod.rs` for the module overview.

use std::sync::{Arc, Mutex, OnceLock};

use tinyflows::browser::BrowserRelay;
use tinyflows::companion::{CompanionServer, CompanionServerConfig, RelayPolicy};

use crate::openhuman::browser_companion::store::resolve_secret_store;
use crate::openhuman::browser_companion::types::{BrowserCompanionStatus, PairingInfo};
use crate::openhuman::browser_companion::LOG_PREFIX;
use crate::openhuman::config::Config;
use crate::openhuman::tinyflows::build_capabilities;

/// Namespace passed to [`build_capabilities`] for every non-browser effect
/// (state store, http, code, agent) the companion server's native workflow
/// runs might use in a later increment.
const CAPS_STATE_NAMESPACE: &str = "browser-companion";

/// In-memory lifecycle state for the companion relay: at most one instance
/// runs per process.
struct CompanionRuntime {
    server: Option<CompanionServer>,
    task: Option<tokio::task::JoinHandle<()>>,
    /// The extension id the running relay was actually started with. This is
    /// the authoritative source for status + restart — NOT `Config`, which may
    /// hold a stale/default value after an unpersisted `pair` (Stage E persists
    /// it; until then only the runtime knows the live id).
    active_extension_id: Option<String>,
}

impl CompanionRuntime {
    const fn empty() -> Self {
        Self {
            server: None,
            task: None,
            active_extension_id: None,
        }
    }

    /// True only when a server is stored AND its listener task is still alive.
    /// A task that already finished means `serve()` exited (e.g. the internal
    /// bind failed) — so a stored `server` alone must NOT be reported running.
    fn is_running(&self) -> bool {
        self.server.is_some() && self.task.as_ref().is_some_and(|task| !task.is_finished())
    }

    /// Drops a server whose listener task has already exited, so a later start
    /// is not blocked by dead state left behind by a failed `serve()`.
    fn reap_if_dead(&mut self) {
        if self.server.is_some()
            && self
                .task
                .as_ref()
                .is_some_and(tokio::task::JoinHandle::is_finished)
        {
            log::warn!(
                "{LOG_PREFIX} reap_if_dead: listener task exited unexpectedly; clearing stale runtime state"
            );
            self.server = None;
            self.task = None;
            self.active_extension_id = None;
        }
    }
}

static RUNTIME: OnceLock<Mutex<CompanionRuntime>> = OnceLock::new();

fn runtime() -> &'static Mutex<CompanionRuntime> {
    RUNTIME.get_or_init(|| Mutex::new(CompanionRuntime::empty()))
}

/// Runs `f` against the live server, but ONLY when the relay is genuinely
/// running — first reaping any listener task that has already exited. Returns
/// `None` when the relay is not running (never started, stopped, or its
/// `serve()` died), so every public runtime observation honors listener
/// liveness rather than reading a stale `server` handle directly.
fn with_live_server<T>(f: impl FnOnce(&CompanionServer) -> T) -> Option<T> {
    let mut guard = runtime()
        .lock()
        .expect("browser_companion runtime poisoned");
    guard.reap_if_dead();
    if !guard.is_running() {
        return None;
    }
    guard.server.as_ref().map(f)
}

fn workflows_dir(config: &Config) -> std::path::PathBuf {
    config
        .workspace_dir
        .join("browser_companion")
        .join("workflows")
}

fn relay_url(port: u16) -> String {
    format!("ws://127.0.0.1:{port}/v1/extension")
}

/// Starts the companion relay if `config.browser_companion.enabled` and no
/// instance is already running. No-op (with a log line) otherwise.
pub async fn start_companion_server(config: &Config) -> anyhow::Result<()> {
    log::debug!("{LOG_PREFIX} start_companion_server: entry");

    if !config.browser_companion.enabled {
        log::debug!("{LOG_PREFIX} start_companion_server: disabled via config; skipping");
        return Ok(());
    }

    start_with_extension_id(config, config.browser_companion.extension_id.clone()).await
}

/// Core start path shared by [`start_companion_server`] (uses the persisted
/// `extension_id`) and [`pair`] (uses the freshly supplied one). Does **not**
/// re-check `config.browser_companion.enabled` — callers that want the
/// enabled-gate must go through [`start_companion_server`].
async fn start_with_extension_id(config: &Config, extension_id: String) -> anyhow::Result<()> {
    {
        let mut guard = runtime()
            .lock()
            .expect("browser_companion runtime poisoned");
        // A previous `serve()` may have exited (bind failure); clear that dead
        // state so it does not masquerade as "already running" and block us.
        guard.reap_if_dead();
        if guard.is_running() {
            log::debug!("{LOG_PREFIX} start_with_extension_id: already running; no-op");
            return Ok(());
        }
    }

    let port = config.browser_companion.port;
    log::info!(
        "{LOG_PREFIX} start_with_extension_id: starting relay port={port} extension_id_set={}",
        !extension_id.is_empty()
    );

    let policy = RelayPolicy::loopback(port);
    let workflows_dir = workflows_dir(config);
    std::fs::create_dir_all(&workflows_dir).map_err(|error| {
        log::warn!(
            "{LOG_PREFIX} start_companion_server: failed to create workflows dir {}: {error}",
            workflows_dir.display()
        );
        anyhow::anyhow!("failed to create browser_companion workflows dir: {error}")
    })?;

    let secret_store = resolve_secret_store(config)?;
    let pairing_secret = secret_store.load_or_create().map_err(|error| {
        log::warn!("{LOG_PREFIX} start_companion_server: secret load_or_create failed: {error}");
        anyhow::anyhow!("failed to load or create pairing secret: {error}")
    })?;

    let capabilities = build_capabilities(Arc::new(config.clone()), CAPS_STATE_NAMESPACE);

    // Keep the id we started with as the authoritative active id (Config may
    // not yet hold it — `pair` doesn't persist until Stage E).
    let active_extension_id = extension_id.clone();

    let server_config = CompanionServerConfig {
        policy,
        extension_id,
        pairing_secret,
        workflows_dir,
        capabilities,
    };

    let server = CompanionServer::new(server_config).map_err(|error| {
        log::warn!("{LOG_PREFIX} start_companion_server: CompanionServer::new failed: {error}");
        anyhow::anyhow!("failed to construct companion server: {error}")
    })?;

    let bind_addr = server.bind_addr();
    let serving = server.clone();
    let task = tokio::spawn(async move {
        log::info!("{LOG_PREFIX} companion relay serving on {bind_addr}");
        if let Err(error) = serving.serve().await {
            log::error!("{LOG_PREFIX} companion relay listener exited with error: {error}");
        } else {
            log::info!("{LOG_PREFIX} companion relay stopped cleanly");
        }
    });

    let mut guard = runtime()
        .lock()
        .expect("browser_companion runtime poisoned");
    guard.server = Some(server);
    guard.task = Some(task);
    guard.active_extension_id = Some(active_extension_id);
    log::info!("{LOG_PREFIX} start_with_extension_id: relay started bind_addr={bind_addr}");
    Ok(())
}

/// Stops the companion relay if running. No-op (with a log line) otherwise.
pub async fn stop_companion_server() {
    log::debug!("{LOG_PREFIX} stop_companion_server: entry");
    let (server, task) = {
        let mut guard = runtime()
            .lock()
            .expect("browser_companion runtime poisoned");
        guard.active_extension_id = None;
        (guard.server.take(), guard.task.take())
    };

    if server.is_none() {
        log::debug!("{LOG_PREFIX} stop_companion_server: not running; no-op");
        return;
    }

    if let Some(task) = task {
        task.abort();
        // Await the aborted task so its `TcpListener` is actually dropped before
        // we return. Without this, an immediate pair/rotate restart can lose the
        // race to re-bind the same loopback port (`Address already in use`).
        let _ = task.await;
        log::info!("{LOG_PREFIX} stop_companion_server: relay task aborted and joined");
    }
}

/// Returns a handle usable to route `slug:"browser"` tool calls to the
/// paired extension, for later flows wiring (Stage C3). `None` when the
/// relay is not running.
pub fn browser_relay() -> Option<Arc<dyn BrowserRelay>> {
    with_live_server(CompanionServer::browser_relay)
}

/// Binds a workflow run to an explicitly-shared browser tab so that run's
/// `slug:"browser"` tool calls (routed through the handle from
/// [`browser_relay`]) are authorized against that tab. Mirrors
/// `tinyflows::companion::CompanionServer::bind_run`, keeping the server
/// handle itself encapsulated in this domain (Stage C3 — flows wiring).
///
/// Returns an error when the relay is not currently running, or when the
/// underlying `CompanionServer::bind_run` call rejects the binding (e.g. the
/// tab isn't one the extension has explicitly shared — `tab_not_shared`).
pub fn bind_run(run_id: &str, tab_id: u64) -> anyhow::Result<()> {
    log::debug!("{LOG_PREFIX} bind_run: entry run_id={run_id} tab_id={tab_id}");
    let Some(result) = with_live_server(|server| server.bind_run(run_id.to_string(), tab_id))
    else {
        log::warn!("{LOG_PREFIX} bind_run: relay not running; cannot bind run_id={run_id}");
        return Err(anyhow::anyhow!(
            "browser companion relay is not running; cannot bind run '{run_id}' to tab {tab_id}"
        ));
    };
    result.map_err(|error| {
        log::warn!(
            "{LOG_PREFIX} bind_run: CompanionServer::bind_run failed run_id={run_id} \
             tab_id={tab_id}: {error}"
        );
        anyhow::anyhow!("failed to bind run '{run_id}' to browser tab {tab_id}: {error}")
    })?;
    log::info!("{LOG_PREFIX} bind_run: bound run_id={run_id} tab_id={tab_id}");
    Ok(())
}

/// Releases a run→tab binding after an external run settles. No-op (with a
/// debug log) if the relay isn't running or nothing was bound for `run_id` —
/// idempotent, mirroring `tinyflows::companion::CompanionServer::unbind_run`.
pub fn unbind_run(run_id: &str) {
    log::debug!("{LOG_PREFIX} unbind_run: entry run_id={run_id}");
    if with_live_server(|server| server.unbind_run(run_id)).is_none() {
        log::debug!("{LOG_PREFIX} unbind_run: relay not running; no-op");
        return;
    }
    log::debug!("{LOG_PREFIX} unbind_run: unbound run_id={run_id} (no-op if it wasn't bound)");
}

/// Whether a paired extension currently holds an authenticated relay
/// session. Always `false` when the relay is not running.
pub fn is_extension_connected() -> bool {
    with_live_server(CompanionServer::is_extension_connected).unwrap_or(false)
}

/// Current lifecycle + pairing snapshot.
pub fn companion_status(config: &Config) -> BrowserCompanionStatus {
    let mut guard = runtime()
        .lock()
        .expect("browser_companion runtime poisoned");
    // Don't report a relay whose listener task already died as running.
    guard.reap_if_dead();
    let running = guard.is_running();
    // Only observe the server when it's genuinely running — consistent with the
    // public accessors, which all gate on liveness via `with_live_server`.
    let live_server = running.then(|| guard.server.as_ref()).flatten();
    let extension_connected = live_server
        .map(CompanionServer::is_extension_connected)
        .unwrap_or(false);
    let shared_tabs: Vec<_> = live_server
        .map(|server| server.shared_tabs().into_iter().map(Into::into).collect())
        .unwrap_or_default();

    // Prefer the id the running relay was actually started with (the live
    // truth); fall back to Config only when nothing is running (Stage E will
    // persist the paired id so it survives a restart).
    let paired_extension_id = guard.active_extension_id.clone().or_else(|| {
        Some(config.browser_companion.extension_id.clone())
            .filter(|extension_id| !extension_id.is_empty())
    });
    let relay_url = running.then(|| relay_url(config.browser_companion.port));

    log::debug!(
        "{LOG_PREFIX} companion_status: running={running} extension_connected={extension_connected} shared_tab_count={}",
        shared_tabs.len()
    );

    BrowserCompanionStatus {
        running,
        extension_connected,
        paired_extension_id,
        relay_url,
        shared_tabs,
    }
}

/// Pairs a new extension id: restarts the relay bound to `extension_id`
/// (rather than whatever is currently persisted in `config`) and returns the
/// relay URL + current pairing secret.
///
/// `config.browser_companion.extension_id` is **not** mutated here (Stage E
/// persists it via `config.update_*` — see `TODO(stage-E)`). The live id is
/// instead retained in `CompanionRuntime::active_extension_id`, which
/// [`companion_status`] and [`rotate_secret`] read as the authoritative
/// source — so status and secret-rotation stay correct across an unpersisted
/// pair, not just until the next process restart.
pub async fn pair(config: &Config, extension_id: String) -> anyhow::Result<PairingInfo> {
    log::info!(
        "{LOG_PREFIX} pair: entry extension_id_len={}",
        extension_id.len()
    );
    // TODO(stage-E): persist `browser_companion.extension_id` via a
    // `config.update_*` RPC once the RPC surface for this domain lands.

    stop_companion_server().await;
    start_with_extension_id(config, extension_id).await?;

    let secret_store = resolve_secret_store(config)?;
    let pairing_secret = secret_store.load_or_create().map_err(|error| {
        log::warn!("{LOG_PREFIX} pair: secret load_or_create failed: {error}");
        anyhow::anyhow!("failed to load pairing secret after pairing: {error}")
    })?;

    log::info!("{LOG_PREFIX} pair: relay restarted with new extension_id");
    Ok(PairingInfo {
        relay_url: relay_url(config.browser_companion.port),
        pairing_secret: pairing_secret.expose().to_string(),
    })
}

/// Clears the pairing (rotates the secret, invalidating the old one) and
/// stops the relay.
///
/// Same in-memory-only caveat as [`pair`] applies to persisting a cleared
/// `extension_id` — see `TODO(stage-E)`.
pub async fn unpair(config: &Config) -> anyhow::Result<()> {
    log::info!("{LOG_PREFIX} unpair: entry");
    // TODO(stage-E): persist the cleared `extension_id` via `config.update_*`.

    let secret_store = resolve_secret_store(config)?;
    secret_store.rotate().map_err(|error| {
        log::warn!("{LOG_PREFIX} unpair: secret rotate failed: {error}");
        anyhow::anyhow!("failed to rotate pairing secret during unpair: {error}")
    })?;

    stop_companion_server().await;
    log::info!("{LOG_PREFIX} unpair: relay stopped and secret rotated");
    Ok(())
}

/// Rotates the pairing secret and restarts the relay (if it was running) so
/// the new secret takes effect, returning it.
pub async fn rotate_secret(config: &Config) -> anyhow::Result<PairingInfo> {
    log::info!("{LOG_PREFIX} rotate_secret: entry");
    let secret_store = resolve_secret_store(config)?;
    let pairing_secret = secret_store.rotate().map_err(|error| {
        log::warn!("{LOG_PREFIX} rotate_secret: secret rotate failed: {error}");
        anyhow::anyhow!("failed to rotate pairing secret: {error}")
    })?;

    // Capture the live extension id BEFORE stopping so the restart rebinds to
    // the same id — NOT `config.extension_id`, which may be stale/default after
    // an unpersisted `pair`. Restarting via `start_companion_server` would also
    // wrongly re-check `config.enabled` and no-op after such a pair.
    let active_extension_id = {
        let mut guard = runtime()
            .lock()
            .expect("browser_companion runtime poisoned");
        guard.reap_if_dead();
        guard
            .is_running()
            .then(|| guard.active_extension_id.clone().unwrap_or_default())
    };
    if let Some(extension_id) = active_extension_id {
        stop_companion_server().await;
        start_with_extension_id(config, extension_id).await?;
        log::info!("{LOG_PREFIX} rotate_secret: relay restarted with rotated secret");
    }

    Ok(PairingInfo {
        relay_url: relay_url(config.browser_companion.port),
        pairing_secret: pairing_secret.expose().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(workspace_dir: std::path::PathBuf) -> Config {
        Config {
            workspace_dir,
            ..Config::default()
        }
    }

    #[test]
    fn status_reports_no_paired_extension_for_default_config() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = test_config(tmp.path().to_path_buf());
        assert!(!config.browser_companion.enabled);
        // `paired_extension_id` now falls back to `config` ONLY when the shared
        // runtime static is idle (it prefers the running relay's live active
        // id). Guard on idle so the lifecycle test (which may run concurrently
        // with a relay up) can't make this flake.
        if browser_relay().is_none() {
            assert_eq!(companion_status(&config).paired_extension_id, None);
        }
    }

    #[test]
    fn companion_status_reports_paired_extension_id_from_config() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut config = test_config(tmp.path().to_path_buf());
        config.browser_companion.extension_id = "abcdefghijklmnopabcdefghijklmnop".to_string();
        // Same idle-guard rationale as above: the config fallback is only the
        // reported value when no relay (with its own active id) is running.
        if browser_relay().is_none() {
            assert_eq!(
                companion_status(&config).paired_extension_id,
                Some("abcdefghijklmnopabcdefghijklmnop".to_string())
            );
        }
    }

    #[test]
    fn relay_url_formats_loopback_websocket_url() {
        assert_eq!(relay_url(32189), "ws://127.0.0.1:32189/v1/extension");
    }

    #[tokio::test]
    async fn start_is_noop_when_disabled_in_config() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = test_config(tmp.path().to_path_buf());
        assert!(!config.browser_companion.enabled);
        // Returns before touching the shared runtime static at all, so this
        // is safe to run alongside any other test in this file.
        let result = start_companion_server(&config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn browser_relay_and_is_extension_connected_never_panic() {
        // Read-only accessors must not panic regardless of whether some
        // other test in this file has a relay running concurrently — they
        // are not asserted against a specific state here.
        let _ = browser_relay();
        let _ = is_extension_connected();
    }

    #[test]
    fn bind_run_errs_when_relay_not_running() {
        // Deliberately does not touch the shared runtime static (no start/stop
        // call), so this is safe to run concurrently with any other test in
        // this file — `bind_run` only reads whether a server is present.
        //
        // NOTE: cannot assert `.is_err()` unconditionally here because this
        // process-wide static may already have a server running from another
        // concurrently-executing test in this file (e.g.
        // `start_then_status_running_then_stop`). Instead assert the
        // documented CONTRACT: when no server is running, `bind_run` errs
        // with a message naming the run id.
        if browser_relay().is_none() {
            let err = bind_run("test-run-not-running", 7)
                .expect_err("bind_run must error when the relay is not running");
            assert!(err.to_string().contains("test-run-not-running"), "{err}");
        }
    }

    #[test]
    fn unbind_run_is_a_noop_when_relay_not_running_or_run_unknown() {
        // Never panics regardless of runtime state; always a safe no-op for
        // an unknown/unbound run id.
        unbind_run("test-run-never-bound");
    }

    /// Full lifecycle: start (bound to an OS-assigned ephemeral port via
    /// port 0, so this can never collide with a real or another test's
    /// port) → status reports running → stop → status reports not running.
    ///
    /// This is the *only* test in this file that calls
    /// `start_companion_server`/`stop_companion_server` with `enabled: true`,
    /// so it cannot race against another test over the shared process-wide
    /// runtime static.
    #[tokio::test]
    async fn start_then_status_running_then_stop() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut config = test_config(tmp.path().to_path_buf());
        config.browser_companion.enabled = true;
        // Port 0: let the OS assign a free ephemeral port. `CompanionServer::new`
        // only validates the policy and builds in-memory state — the actual
        // `TcpListener::bind` happens inside the spawned `serve()` task, so
        // this test's synchronous assertions don't depend on the bind
        // actually completing.
        config.browser_companion.port = 0;
        // Must be exactly 32 chars in a-p to pass `Authenticator::new`'s
        // Chrome-extension-id format check.
        config.browser_companion.extension_id = "abcdefghijklmnopabcdefghijklmnop".to_string();

        start_companion_server(&config)
            .await
            .expect("start_companion_server should succeed with a valid config");

        let status = companion_status(&config);
        assert!(status.running, "relay should report running after start");
        assert!(
            !status.extension_connected,
            "no extension has connected yet"
        );
        assert!(status.shared_tabs.is_empty());
        // The running relay's active id is reported from the runtime (the live
        // authoritative source), matching what we started with.
        assert_eq!(
            status.paired_extension_id.as_deref(),
            Some("abcdefghijklmnopabcdefghijklmnop")
        );

        // Re-pair with a DIFFERENT id: status must reflect the new id even
        // though `config` still holds the old one and is never mutated — the
        // runtime's active id is authoritative. Regression for the lost-id bug.
        let new_id = "ponmlkjihgfedcbaponmlkjihgfedcba";
        let info = pair(&config, new_id.to_string())
            .await
            .expect("pair should restart the relay with the new id");
        assert!(info.relay_url.starts_with("ws://127.0.0.1:"));
        let paired = companion_status(&config);
        assert!(paired.running, "relay should still be running after pair");
        assert_eq!(paired.paired_extension_id.as_deref(), Some(new_id));

        // Rotate the secret: the relay must stay running (restarted with the
        // active id, not the disabled/stale config) and return a fresh secret.
        // Regression for "rotate stops but never restarts after an unpersisted
        // pair".
        let rotated = rotate_secret(&config)
            .await
            .expect("rotate_secret should restart the running relay");
        assert!(!rotated.pairing_secret.is_empty());
        assert!(
            companion_status(&config).running,
            "relay should still be running after rotate_secret"
        );

        // bind_run/unbind_run while the relay is running (still the only test
        // in this file exercising the shared runtime static with a real
        // server up, so no race against another test's start/stop). No tab
        // has been shared by any extension in this test, so binding must fail
        // closed with a `tab_not_shared`-shaped error rather than silently
        // succeeding.
        let err = bind_run("test-run-1", 99).expect_err("no tab is shared in this test");
        assert!(err.to_string().contains("test-run-1"), "{err}");
        // Idempotent no-op even though nothing was ever actually bound.
        unbind_run("test-run-1");

        stop_companion_server().await;

        let status = companion_status(&config);
        assert!(!status.running, "relay should report stopped after stop");
    }
}
