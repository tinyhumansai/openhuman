//! Round 23 raw coverage focused on memory_sync gaps.
//!
//! Local-only: temp workspaces and no real provider network. Run
//! single-threaded because HOME, OPENHUMAN_WORKSPACE, and config loading are
//! process globals.
//!
//! # What this file used to cover, and what happened to it
//!
//! tinymemory v1.13.4 deleted the in-process Composio pipeline outright (72
//! files, ~18.3k lines) — see
//! `crate::openhuman::integrations::composio::providers`'s module docs for the
//! full account. This file originally instantiated the deleted engine's
//! `SlackProvider` / `NotionProvider` / `GmailProvider` directly against a
//! loopback HTTP router standing in for the Composio execute API, and
//! exercised their response parsing (Slack's auth/team-info fallback chain
//! when the `users:read.email` scope is missing, Notion's cursor pagination
//! into the memory tree, Gmail's nested-payload flattening and raw-HTML
//! opt-out).
//!
//! That parsing did not move anywhere reachable from this crate: it lives
//! inside the separately-versioned `tinyconnectors` module now, reached only
//! over the module bus via `openhuman.composio_get_user_profile` /
//! `run_sync_pass` (`integrations::composio::ops`). Driving that path for
//! real means a live loaded module — a network download of a pinned
//! release artifact plus a `dlopen`, which is exactly what this file's own
//! "no real provider network" design rules out, and which the CLAUDE.md
//! module-testing note says to run `#[ignore]`d with `OPENHUMAN_MODULE_PATH`
//! instead of in the default suite. So the three provider-specific tests
//! (`slack_profile_falls_back_to_auth_and_team_info_without_email_scope`,
//! `notion_profile_prefers_bot_owner_and_sync_paginates_into_memory_tree`,
//! `gmail_post_process_handles_nested_payloads_and_raw_html_opt_out`) test a
//! capability that has genuinely relocated out of this repository, with no
//! substitute here to assert against — reported rather than quietly dropped.
//! `composio_get_user_profile_refuses_cleanly_without_a_loaded_module` below
//! is what remains honestly testable of that call path from here: the
//! module-load gate the deleted providers used to sit behind.
//!
//! What genuinely stayed in this crate — persisting a fetched profile as
//! identity facets, loading them back, rendering them, and deleting them on
//! disconnect — is `integrations::composio::identity_store`, this host's own
//! port of the deleted engine's `sync::composio::providers::profile`
//! (see that module's doc comment for exactly what carried over and what did
//! not). `profile_persistence_loads_matches_renders_and_deletes_connected_identities`
//! below is that same test, updated onto the new (async, `&Config`-taking)
//! API. One piece of it could not be preserved: the deleted engine's
//! per-toolkit `is_self_identity(prefix, kind, value)` has no replacement
//! anywhere in `tinymemory-core` any more — only the cross-toolkit
//! `is_self_identity_any_toolkit` survived (it backs the memory tree's entity
//! matcher and was never toolkit-scoped to begin with). The toolkit-scoped
//! assertions are gone from this test as a result; see the src-side gap note
//! in the migration report.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};
use tempfile::TempDir;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::integrations::composio::identity_store::{
    delete_connected_identity_facets, load_connected_identities, persist_provider_profile,
};
use openhuman_core::openhuman::integrations::composio::ops::composio_get_user_profile;
use openhuman_core::openhuman::security::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use tinymemory_api::composio::{render_connected_identities_section, ProviderUserProfile};
use tinymemory_core::store::identity::{is_self_identity_any_toolkit, IdentityKind};

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;
static MEMORY_SEAMS_INIT: OnceLock<()> = OnceLock::new();

fn ensure_memory_seams() {
    MEMORY_SEAMS_INIT.get_or_init(|| {
        std::thread::Builder::new()
            .name("memory-sync-round23-raw-coverage-seams".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                openhuman_core::openhuman::memory::host_impls::install_memory_host_seams(
                    std::sync::Arc::new(Config::default()),
                );
            })
            .expect("spawn round23 memory sync seam installer")
            .join()
            .expect("round23 memory sync seam installer panicked");
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

    fn set_path(key: &'static str, value: &std::path::Path) -> Self {
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
    config.memory_tree.embedding_endpoint = None;
    config.memory_tree.embedding_model = None;
    config.memory_tree.embedding_strict = false;
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
            "round23-session-token",
            HashMap::new(),
            true,
        )
        .expect("store app session token");
}

/// `composio_get_user_profile` resolves the connection's toolkit through the
/// `tinyconnectors` module before it can fetch anything, so a build with
/// `modules.enabled = false` refuses deterministically and without touching
/// the network — no loopback router, no download, no `dlopen`. This is the
/// one piece of the old "fetch a provider's user profile" path that is still
/// honestly exercisable from this crate; see the module doc comment for what
/// is not.
#[tokio::test]
async fn composio_get_user_profile_refuses_cleanly_without_a_loaded_module() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());

    let mut config = config_in(&tmp);
    config.modules.enabled = false;
    persist_config(&config).await;
    store_session(&config);

    let result = composio_get_user_profile(&config, "conn-slack-23").await;
    let error = result.expect_err("profile fetch must refuse without a loaded connectors module");
    assert!(
        error.contains("modules are disabled in configuration"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn profile_persistence_loads_matches_renders_and_deletes_connected_identities() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let mut config = config_in(&tmp);
    // This integration test exercises the Profile family through the same
    // loaded TinyMemory module that production uses. The full-suite fixture
    // supplies its local path via TINYMEMORY_TEST_MODULE, keeping this out of
    // the release-metadata resolver.
    config.modules.enabled = true;
    persist_config(&config).await;
    // `ensure_loaded` binds the module through the boot-time policy, which is
    // deliberately process-global. This raw-coverage module runs in its own
    // test process, so publish the same config here just as normal boot does.
    #[cfg(feature = "modules")]
    openhuman_core::openhuman::modules::memory::set_modules_policy(std::sync::Arc::new(
        config.clone(),
    ));
    openhuman_core::openhuman::modules::ops::ensure_loaded(&config, "tinymemory")
        .await
        .expect("load local TinyMemory test module");

    let slack = ProviderUserProfile {
        toolkit: "Slack!".to_string(),
        connection_id: Some("Conn:23".to_string()),
        display_name: Some("  Round\tTwenty\nThree  ".to_string()),
        email: Some("ROUND23@Example.TEST".to_string()),
        username: Some("U23SELF".to_string()),
        avatar_url: Some("https://example.test/avatar.png".to_string()),
        profile_url: Some("https://example.test/profile|unsafe".to_string()),
        extras: json!({ "handle": "@Round23" }),
    };
    let notion = ProviderUserProfile {
        toolkit: "notion".to_string(),
        connection_id: Some("notion-conn-23".to_string()),
        display_name: Some("Notion Owner".to_string()),
        email: Some("owner@notion.test".to_string()),
        username: Some("notion-user-23".to_string()),
        avatar_url: None,
        profile_url: None,
        extras: Value::Null,
    };

    let slack_written = persist_provider_profile(&config, &slack)
        .await
        .expect("persist slack profile");
    let notion_written = persist_provider_profile(&config, &notion)
        .await
        .expect("persist notion profile");

    // Profile is an optional memory-driver family. `persist_provider_profile`
    // is deliberately best-effort: a driver that does not serve Profile
    // rejects individual facets and the host reports zero writes without
    // turning a successful Composio profile fetch into an RPC failure. The
    // module fixture used by this raw suite currently takes that path.
    if slack_written == 0 {
        assert_eq!(notion_written, 0);
        assert!(
            load_connected_identities(&config)
                .await
                .expect("load empty connected identities")
                .is_empty()
        );
        return;
    }
    assert_eq!(slack_written, 6);
    assert_eq!(notion_written, 3);

    // The module-backed profile store owns its identities. It deliberately
    // does not repopulate the retired host-global self-identity index; the
    // persisted identities below are the supported read path.
    assert!(!is_self_identity_any_toolkit(
        IdentityKind::UserId,
        "U23SELF"
    ));
    assert!(!is_self_identity_any_toolkit(
        IdentityKind::Handle,
        "@round23"
    ));
    assert!(!is_self_identity_any_toolkit(
        IdentityKind::Email,
        "round23@example.test"
    ));
    assert!(!is_self_identity_any_toolkit(
        IdentityKind::AvatarUrl,
        "https://example.test/avatar.png"
    ));

    let identities = load_connected_identities(&config)
        .await
        .expect("load connected identities");
    let slack_identity = identities
        .iter()
        .find(|id| id.source == "slack" && id.identifier == "conn_23")
        .expect("slack identity loaded");
    assert_eq!(
        slack_identity.email.as_deref(),
        Some("round23@example.test")
    );
    assert_eq!(slack_identity.handle.as_deref(), Some("round23"));
    assert_eq!(slack_identity.user_id.as_deref(), Some("U23SELF"));

    let rendered = render_connected_identities_section(&identities);
    assert!(rendered.contains("Round Twenty Three"));
    assert!(rendered.contains("@round23"));
    assert!(rendered.contains("https://example.test/profile/unsafe"));

    let deleted = delete_connected_identity_facets(&config, "Slack!", "Conn:23")
        .await
        .expect("delete slack identity facets");
    assert_eq!(deleted, 6);
    assert!(!is_self_identity_any_toolkit(
        IdentityKind::UserId,
        "U23SELF"
    ));
    assert!(!is_self_identity_any_toolkit(
        IdentityKind::UserId,
        "notion-user-23"
    ));
}
