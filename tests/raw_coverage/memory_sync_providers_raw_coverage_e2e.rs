//! Focused raw coverage for Composio memory-sync providers.
//!
//! Local-only: temp workspaces, no real provider network. Run with
//! `--test-threads=1` because config, HOME, and OPENHUMAN_WORKSPACE are
//! process globals.
//!
//! # What this file used to cover, and why it is now mostly a gap report
//!
//! tinymemory v1.13.4 deleted the entire in-process Composio provider
//! registry outright (72 files, ~18.3k lines) — see
//! `crate::openhuman::integrations::composio::providers`'s module docs for
//! the full account. This file used to instantiate the deleted engine's
//! per-toolkit providers (`SlackProvider`, `GmailProvider`, `NotionProvider`,
//! `GitHubProvider`, `LinearProvider`, `ClickUpProvider`) directly against a
//! loopback HTTP router standing in for the Composio execute API, and
//! exercised, per toolkit:
//!
//! - `max_items=N` capping ingestion to exactly N items mid-page (not just a
//!   page-level cap) — one test per toolkit.
//! - `sync_depth_days` injecting a provider-specific date floor into the
//!   outbound query (Gmail's `after:`, GitHub's `updated:>`) or filtering
//!   client-side against a descending-order page (Notion, Linear, ClickUp's
//!   epoch-ms `date_updated`).
//! - Gmail's `stop_on_empty_pending` early-stop optimization: a page that is
//!   already fully synced (per a pre-seeded `SyncState`) halts pagination
//!   after one round trip instead of walking every page up to the ceiling.
//! - `fetch_tasks`/`fetch_user_profile`/`sync` wiring for GitHub and ClickUp,
//!   plus three Composio bus subscribers driven inline.
//!
//! None of that per-provider sync logic (pagination, cap enforcement, date
//! floors, cursor/state bookkeeping) moved anywhere reachable from this
//! crate — it now lives entirely inside the separately-versioned
//! `tinyconnectors` module. Exercising it for real means a live loaded
//! module (a network download of a pinned release plus a `dlopen`), which
//! this file's own "no real provider network" design rules out and which
//! the CLAUDE.md module-testing note says belongs in an `#[ignore]`d test
//! with `OPENHUMAN_MODULE_PATH`, not the default suite. `ProviderContext`,
//! `ComposioProvider`, every concrete provider struct, and the Composio-
//! specific `providers::sync_state::{SyncState, PersistedSyncState}`
//! persistence pair are all gone with nothing in this crate to assert
//! against — reported as a genuine coverage gap rather than silently
//! dropped. (`tinycortex::memory::sync::state::SyncState` still exists, but
//! it is a different, unrelated sync pipeline's state — for GitHub-repo /
//! workspace source sync, not Composio providers — and reusing it here would
//! misrepresent what is actually being tested.)
//!
//! What remains honestly testable of the "fetch a provider profile" /
//! "sync a connection" call path is `composio_get_user_profile` /
//! `composio_sync` (`integrations::composio::ops`), which now refuse
//! cleanly and deterministically — without any network call — when no
//! connectors module is loaded. The three Composio bus subscribers
//! (`memory::sync::composio::bus::*`) are themselves untouched by the
//! deletion, so their wiring (name/domains, and that `handle()` returns
//! without panicking) is preserved below too. See
//! `memory_sync_slack_bus_raw_coverage_e2e.rs` for the sibling test that
//! established this same pattern in more detail (including why
//! `ComposioConnectionCreatedSubscriber` must not be driven while signed in,
//! to avoid a real backend network poll).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use serde_json::json;
use tempfile::TempDir;

use openhuman_core::core::events::DomainEvent;
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::integrations::composio::ops::{composio_get_user_profile, composio_sync};
use openhuman_core::openhuman::memory::sync::composio::bus::{
    ComposioConfigChangedSubscriber, ComposioConnectionCreatedSubscriber, ComposioTriggerSubscriber,
};
use openhuman_core::openhuman::security::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use tinybus::EventHandler;

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;
static MEMORY_SEAMS_INIT: OnceLock<()> = OnceLock::new();

fn ensure_memory_seams() {
    MEMORY_SEAMS_INIT.get_or_init(|| {
        std::thread::Builder::new()
            .name("memory-sync-providers-raw-coverage-seams".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
            })
            .expect("spawn memory sync provider seam installer")
            .join()
            .expect("memory sync provider seam installer panicked");
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

    #[allow(dead_code)]
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
    ensure_memory_seams();
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    // Deterministic, network-free: no loopback router stands in for the
    // Composio execute API any more (see module doc comment above), so
    // every module-mediated call in this file resolves to a clean "modules
    // disabled" refusal instead of attempting a real download.
    config.modules.enabled = false;
    config
}

async fn persist_config(config: &Config) {
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
    config.save().await.expect("save config");
}

#[allow(dead_code)]
fn store_session(config: &Config) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "round17-session-token",
            HashMap::new(),
            true,
        )
        .expect("store app session token");
}

/// What the deleted per-toolkit `fetch_user_profile`/`sync` tests covered
/// for the "reach a connected account" half: `composio_get_user_profile`
/// and `composio_sync` are the current entry points (`integrations::composio::ops`),
/// and both refuse cleanly, deterministically and without touching the
/// network when no connectors module is loaded.
#[tokio::test]
async fn composio_get_user_profile_and_sync_refuse_cleanly_without_a_loaded_module() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());

    let config = config_in(&tmp);
    persist_config(&config).await;

    for toolkit in ["github", "clickup", "gmail", "notion", "slack", "linear"] {
        let connection_id = format!("conn-{toolkit}-round17");
        let error = composio_get_user_profile(&config, &connection_id)
            .await
            .expect_err("profile fetch must refuse without a loaded connectors module");
        assert!(
            error.contains("modules are disabled in configuration"),
            "unexpected error for {toolkit}: {error}"
        );

        // `composio_sync` starts a background task and reports "started"
        // immediately rather than surfacing the eventual failure — see its
        // doc comment (it is a fire-and-forget RPC). It still resolves the
        // toolkit for the connection first, which is the same module-
        // mediated call `composio_get_user_profile` makes, so it refuses at
        // the same point for a connection nothing has registered.
        let outcome = composio_sync(&config, &connection_id, None).await;
        assert!(
            outcome.is_err(),
            "sync must refuse to resolve toolkit for an unregistered connection ({toolkit})"
        );
    }
}

/// The Composio bus subscribers are untouched by the tinymemory v1.13.4
/// deletion — only what their handlers call underneath changed (see module
/// doc comment). This drives all three with `modules.enabled = false` and
/// signed out, so every module- or network-mediated call inside them fails
/// closed and logs rather than reaching out, and asserts the wiring
/// (name/domains) plus that `handle()` returns without panicking.
#[tokio::test]
async fn composio_bus_subscribers_wire_up_and_return_without_a_loaded_module() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());

    let config = config_in(&tmp);
    persist_config(&config).await;

    let trigger_sub = ComposioTriggerSubscriber::new();
    assert_eq!(trigger_sub.name(), "composio::trigger");
    assert_eq!(trigger_sub.domains().unwrap(), &["composio"]);
    trigger_sub
        .handle(&DomainEvent::ComposioTriggerReceived {
            toolkit: "github".to_string(),
            trigger: "GITHUB_ISSUE_OPENED".to_string(),
            metadata_id: "id-round17".to_string(),
            metadata_uuid: "uuid-round17".to_string(),
            payload: json!({ "text": "hello" }),
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

    // Deliberately signed out: `ComposioConnectionCreatedSubscriber::handle`
    // spawns a detached task that would otherwise reach a real backend via
    // `wait_for_connection_active` once past its `create_composio_client`
    // guard. Staying signed out fails that guard closed immediately.
    let connection_sub = ComposioConnectionCreatedSubscriber::new();
    assert_eq!(connection_sub.name(), "composio::connection_created");
    assert_eq!(connection_sub.domains().unwrap(), &["composio"]);
    connection_sub
        .handle(&DomainEvent::ComposioConnectionCreated {
            toolkit: "github".to_string(),
            connection_id: "conn-github-round17".to_string(),
            connect_url: "https://github.com/login/oauth/authorize".to_string(),
        })
        .await;

    // Give the connection-created handler's detached `tokio::spawn` a beat
    // to run and fail closed, so a panic inside that task has a chance to
    // surface before the test process exits.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}
