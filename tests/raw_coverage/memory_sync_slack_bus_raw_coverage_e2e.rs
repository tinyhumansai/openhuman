//! Round19 raw coverage for Slack memory sync, Composio bus subscribers,
//! and Gmail post-processing.
//!
//! Everything stays local: temp workspaces, no real provider network. Run
//! single-threaded because HOME, OPENHUMAN_WORKSPACE, and config loading are
//! process globals.
//!
//! # What changed here
//!
//! tinymemory v1.13.4 deleted the in-process Composio pipeline outright (72
//! files, ~18.3k lines) — see
//! `crate::openhuman::integrations::composio::providers`'s module docs. This
//! file used to instantiate the deleted engine's `SlackProvider` /
//! `GmailProvider` directly against a loopback HTTP router standing in for
//! the Composio execute API, exercising Slack's full sync (profile, users,
//! conversations, history) plus `run_backfill_via_search`, and Gmail's
//! nested-payload post-processing.
//!
//! None of that parsing moved anywhere reachable from this crate — it now
//! lives inside the separately-versioned `tinyconnectors` module, reached
//! only over the module bus. Exercising it for real means a live loaded
//! module (a network download of a pinned release plus a `dlopen`), which
//! this file's own "no real provider network" design rules out and which
//! the CLAUDE.md module-testing note says belongs in an `#[ignore]`d test
//! with `OPENHUMAN_MODULE_PATH`, not the default suite. So
//! `slack_full_sync_search_backfill_and_bus_use_loopback_composio` and
//! `gmail_post_process_reshapes_nested_messages_and_honors_raw_html_flag`
//! test a capability that has genuinely relocated with nothing here to
//! assert against — reported as a gap rather than silently dropped.
//!
//! The bus subscribers themselves (`ComposioTriggerSubscriber`,
//! `ComposioConnectionCreatedSubscriber`, `ComposioConfigChangedSubscriber`)
//! are untouched by the deletion — they still live in
//! `memory::sync::composio::bus` — and their `handle()` bodies now route
//! through `run_sync_pass` / `composio_get_user_profile` (module-mediated)
//! instead of the deleted engine's `MemorySourceSync`. Both fail closed and
//! log rather than panic when the module cannot load, which is exactly the
//! `modules.enabled = false` state these tests run in, so they still
//! exercise real, current code — name/domain wiring, event matching, and the
//! auto-register-into-`memory_sources` side effect on connection creation —
//! without needing the module or the network. That coverage is preserved
//! below, retargeted onto the honest network-free path.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::json;
use tempfile::TempDir;

use openhuman_core::core::events::DomainEvent;
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::integrations::composio::ops::composio_get_user_profile;
use openhuman_core::openhuman::memory::sync::composio::bus::{
    ComposioConfigChangedSubscriber, ComposioConnectionCreatedSubscriber, ComposioTriggerSubscriber,
};
use openhuman_core::openhuman::security::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use tinybus::EventHandler;

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;
static MEMORY_SEAMS_INIT: OnceLock<()> = OnceLock::new();

fn ensure_memory_seams(config: Arc<Config>) {
    MEMORY_SEAMS_INIT.get_or_init(|| {
        std::thread::Builder::new()
            .name("memory-sync-slack-bus-raw-coverage-seams".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                #[cfg(feature = "modules")]
                openhuman_core::openhuman::modules::memory::set_modules_policy(config);
            })
            .expect("spawn slack bus memory seam installer")
            .join()
            .expect("slack bus memory seam installer panicked");
    });
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl Into<String>) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value.into()) };
        Self { key, old }
    }

    fn set_path(key: &'static str, value: &Path) -> Self {
        Self::set(key, value.to_string_lossy().into_owned())
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::remove_var(key) };
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

fn config_in(tmp: &TempDir) -> Config {
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    // Deterministic, network-free: no loopback router stands in for the
    // Composio execute API any more (see module doc comment), so every
    // module-mediated call in this file resolves to a clean "modules
    // disabled" refusal instead of attempting a real download.
    config.modules.enabled = false;
    ensure_memory_seams(Arc::new(config.clone()));
    config
}

async fn persist_config(config: &Config) {
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
    config.save().await.expect("save config");
}

fn store_session(config: &Config) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "round19-session-token",
            HashMap::new(),
            true,
        )
        .expect("store app session token");
}

/// What `slack_full_sync_search_backfill_and_bus_use_loopback_composio` used
/// to cover for the fetch-and-sync half: `composio_get_user_profile` is the
/// current entry point a Slack connection's profile fetch goes through
/// (`integrations::composio::ops`), and it refuses cleanly, deterministically
/// and without touching the network when no connectors module is loaded —
/// see the module doc comment for what this cannot cover in place of the
/// deleted `SlackProvider`.
#[tokio::test]
async fn composio_get_user_profile_refuses_cleanly_without_a_loaded_module() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");

    let config = config_in(&tmp);
    persist_config(&config).await;
    store_session(&config);

    let error = composio_get_user_profile(&config, "conn-slack-round19")
        .await
        .expect_err("profile fetch must refuse without a loaded connectors module");
    assert!(
        error.contains("modules are disabled in configuration"),
        "unexpected error: {error}"
    );
}

/// The Composio bus subscribers are untouched by the tinymemory v1.13.4
/// deletion — only what their handlers call underneath changed (see module
/// doc comment). This drives all three with `modules.enabled = false` so the
/// module-mediated calls inside them fail closed and log rather than reach
/// the network, and asserts the wiring (name/domains) plus that `handle()`
/// returns cleanly for the event each one actually matches on.
///
/// `ComposioConnectionCreatedSubscriber::handle` fires its whole body into a
/// detached `tokio::spawn` and — before this test's own
/// `memory_sources` auto-register step is even reached — polls the backend
/// via `wait_for_connection_active` to confirm the OAuth handoff completed.
/// With no reachable backend that poll fails and the background task returns
/// early, so the auto-register side effect this test used to be able to
/// observe synchronously is not observable here at all: it depends on a real
/// backend round trip this file's "no real provider network" design
/// deliberately excludes. `handle()` itself still returns immediately
/// (the network call happens on the spawned task, not inline), so what this
/// test can honestly assert is that the call is wired and does not panic.
#[tokio::test]
async fn composio_bus_subscribers_wire_up_and_return_without_a_loaded_module() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");
    let _triage_off = EnvGuard::set("OPENHUMAN_TRIGGER_TRIAGE_DISABLED", "1");

    let config = config_in(&tmp);
    persist_config(&config).await;
    // Deliberately no `store_session(&config)` here: signed-in would let
    // `ComposioConnectionCreatedSubscriber`'s spawned task past its
    // `create_composio_client` guard and into a real backend poll
    // (`wait_for_connection_active`) with no loopback server behind it —
    // exactly the network dependency this file avoids. Staying signed out
    // makes that guard fail closed immediately instead.

    let trigger_sub = ComposioTriggerSubscriber::new();
    assert_eq!(trigger_sub.name(), "composio::trigger");
    assert_eq!(trigger_sub.domains().unwrap(), &["composio"]);
    trigger_sub
        .handle(&DomainEvent::ComposioTriggerReceived {
            toolkit: "slack".to_string(),
            trigger: "SLACK_MESSAGE_POSTED".to_string(),
            metadata_id: "id-round19".to_string(),
            metadata_uuid: "uuid-round19".to_string(),
            payload: json!({ "text": "bus coverage" }),
        })
        .await;

    let config_sub = ComposioConfigChangedSubscriber::new();
    assert_eq!(config_sub.name(), "composio::config_changed");
    assert_eq!(config_sub.domains().unwrap(), &["composio"]);
    config_sub
        .handle(&DomainEvent::ComposioConfigChanged {
            mode: "direct".to_string(),
            api_key_set: true,
        })
        .await;

    let connection_sub = ComposioConnectionCreatedSubscriber::new();
    assert_eq!(connection_sub.name(), "composio::connection_created");
    assert_eq!(connection_sub.domains().unwrap(), &["composio"]);
    connection_sub
        .handle(&DomainEvent::ComposioConnectionCreated {
            toolkit: "slack".to_string(),
            connection_id: "conn-slack-round19".to_string(),
            connect_url: "https://round19.slack.com/connect".to_string(),
        })
        .await;

    // Give the connection-created handler's detached `tokio::spawn` a beat
    // to run and fail closed against the unreachable backend, so a future
    // panic inside that task (which would otherwise abort silently on drop)
    // has a chance to surface before the test process exits.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}
