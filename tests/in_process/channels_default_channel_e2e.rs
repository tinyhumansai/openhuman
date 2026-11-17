//! Default messaging channel: `channels_set_default` / `channels_get_default`.
//!
//! # Why this target exists
//!
//! Both methods are in the domain-e2e gate's uncovered list
//! (`scripts/check-domain-e2e-coverage.mjs`), which counts only
//! `tests/**/*_e2e.rs` targets. Before this file the complete picture was:
//!
//! * `app/src/services/api/channelConnectionsApi.ts:255`, `:263` — **shipped
//!   frontend code** calls both.
//! * `app/src/services/api/channelConnectionsApi.test.ts:47-60` — a Vitest that
//!   asserts the client *emits the right method name* against a mocked
//!   transport. It never reaches the core, so it would keep passing if both
//!   controllers were deleted tomorrow.
//! * Four WDIO/Playwright specs call `channels_set_default` as **setup** and
//!   assert nothing about it.
//!
//! So the round trip had no coverage anywhere above a mocked transport.
//!
//! # What is actually worth asserting
//!
//! A bare set→get round trip is the weak half. `channels/proactive.rs:100-116`
//! documents the interesting invariant: `channels_set_default` mutates the live
//! proactive subscriber's `active_channel` handle **in place** via
//! `set_runtime_active_channel`, "so a default-channel switch from the UI takes
//! effect without a restart" (issue #3712, "switch default channel
//! Telegram<->Discord"), and `proactive.rs:226-240` reads that handle back on
//! every proactive delivery. The failure that bug describes is: config
//! persists, the live handle does not update, proactive messages keep going to
//! the old channel until the process restarts — and a test that only reads the
//! config back cannot see it.
//!
//! **That assertion IS made here** — see
//! `set_default_applies_to_the_live_proactive_handle`. An earlier revision of
//! this file omitted it, on the reasoning that `proactive.rs:135-138` says
//! `set_runtime_active_channel` is "a no-op when no subscriber has registered a
//! handle (e.g. unit tests)", so the assertion would pass vacuously. **That was
//! wrong**: `register_active_channel_handle` (`proactive.rs:129`) is `pub`, so
//! the test registers its own handle and no channel runtime is needed. The
//! non-vacuity control is asserting the handle reads `None` *before* the call —
//! without it, "still unset afterwards" would look like a pass.
//!
//! So this target asserts the persistence and live-apply halves over the wire.
//! The op-level round trip, canonicalisation and `"web"` fallback live in
//! `channels/controllers/ops_connect_status_tests.rs`, and the get/set wire
//! shapes in `domain_modules_e2e`.
//!
//! No network: `api_url` points at a closed port.
//!
//! Run with: `cargo test -p openhuman-cli --test in_process_all`

use crate::env_guard::env_lock_async;
use crate::env_guard::EnvVarGuard;
use crate::rpc_auth::ensure_rpc_auth;
use crate::rpc_harness::rpc;
use std::path::Path;
use std::sync::{Arc, RwLock};

use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_rpc::server::build_core_http_router;

// ── Env isolation ────────────────────────────────────────────────────

fn write_config(openhuman_dir: &Path) {
    std::fs::create_dir_all(openhuman_dir).expect("create .openhuman");
    let cfg = r#"api_url = "http://127.0.0.1:9"
default_model = "channels-default-e2e-model"
default_temperature = 0.2

[secrets]
encrypt = false

[local_ai]
enabled = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0
"#;
    std::fs::write(openhuman_dir.join("config.toml"), cfg).expect("write config.toml");
    let _: openhuman_core::config::Config =
        toml::from_str(cfg).expect("test config must match schema");
}

struct Harness {
    rpc_base: String,
    home: std::path::PathBuf,
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.join.abort();
    }
}

impl Harness {
    /// Every `config.toml` under this harness's `$HOME`, as `(path, body)`.
    ///
    /// Deliberately a search rather than one hard-coded path. The core scopes
    /// config per user (`config/schema/load/dirs.rs` resolves a root
    /// `~/.openhuman` plus a `users/` tree), so `Config::save` does not
    /// necessarily write back to the seed file this harness planted at
    /// `~/.openhuman/config.toml`. Asserting one path would test where this
    /// test *guessed* the file lives rather than whether the choice persisted
    /// — an earlier revision did exactly that and failed against working code.
    fn configs_on_disk(&self) -> Vec<(std::path::PathBuf, String)> {
        fn walk(dir: &Path, out: &mut Vec<(std::path::PathBuf, String)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.file_name().is_some_and(|n| n == "config.toml") {
                    if let Ok(body) = std::fs::read_to_string(&path) {
                        out.push((path, body));
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.home, &mut out);
        out
    }
}

async fn setup() -> Harness {
    ensure_rpc_auth();

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    write_config(&home.join(".openhuman"));

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", &home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
    ];

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr = listener.local_addr().expect("rpc listener addr");
    let router = build_core_http_router(false);
    let join = tokio::spawn(async move { axum::serve(listener, router).await });

    Harness {
        rpc_base: format!("http://{addr}"),
        home,
        _tmp: tmp,
        _guards: guards,
        join,
    }
}

async fn set_default(harness: &Harness, id: i64, channel: &str) -> Value {
    rpc(
        &harness.rpc_base,
        id,
        "openhuman.channels_set_default",
        json!({ "channel": channel }),
    )
    .await
}

// ── Tests ────────────────────────────────────────────────────────────

/// The switch must survive as a *persisted* choice, not just an in-memory one.
///
/// `proactive.rs:114-115` says the choice "is also persisted to
/// `config.channels_config.active_channel`, which seeds the handle on next
/// start". If that write is lost, the default silently reverts on restart —
/// which a same-process round trip cannot see.
#[tokio::test]
async fn set_default_persists_the_choice_to_config_on_disk() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let before = harness.configs_on_disk();
    assert!(
        !before.is_empty(),
        "precondition: the harness should have planted at least one config.toml under $HOME"
    );
    assert!(
        before
            .iter()
            .all(|(_, body)| !body.contains("active_channel")),
        "precondition: no config.toml should already name an active_channel, otherwise the \
         assertion below cannot tell a write from a pre-existing value — found: {:?}",
        before.iter().map(|(path, _)| path).collect::<Vec<_>>()
    );

    set_default(&harness, 1, "discord").await;

    let after = harness.configs_on_disk();
    let persisted: Vec<&std::path::PathBuf> = after
        .iter()
        .filter(|(_, body)| body.contains("active_channel"))
        .map(|(path, _)| path)
        .collect();
    assert!(
        !persisted.is_empty(),
        "channels_set_default returned ok but no config.toml under $HOME gained an \
         `active_channel` key, so the choice will not survive a restart (#3712). Searched: {:?}",
        after.iter().map(|(path, _)| path).collect::<Vec<_>>()
    );
    assert!(
        after
            .iter()
            .any(|(_, body)| body.contains("active_channel") && body.contains("discord")),
        "a config.toml gained an `active_channel` key but not the channel that was set — \
         files carrying the key: {persisted:?}"
    );
}

/// #3712 — the live-apply half, and the one that a config-only test cannot see.
///
/// `set_default_channel` (`ops/connect/status.rs:92-99`) does two things: it
/// persists `channels_config.active_channel`, and it calls
/// `set_runtime_active_channel` so proactive routing follows the switch without
/// a restart. The other tests in this file cover the first. If only the second
/// broke, every one of them would still pass while proactive messages kept
/// going to the old channel until the process restarted — which is exactly the
/// bug #3712 describes.
#[tokio::test]
async fn set_default_applies_to_the_live_proactive_handle() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    // The channel runtime is what registers this in production
    // (`start_channels.rs:463`). Registering it here is what makes the
    // assertion below meaningful rather than a no-op; `proactive.rs:129`
    // exposes it for exactly this reason and the latest registration wins.
    let live_channel: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
    openhuman_core::channels::proactive::register_active_channel_handle(Arc::clone(&live_channel));

    // Non-vacuity control. Without it, a `set_runtime_active_channel` that did
    // nothing at all would leave the handle `None` and the assertion below
    // would be indistinguishable from "never ran".
    assert_eq!(
        live_channel.read().expect("read live handle").clone(),
        None,
        "precondition: the freshly registered handle must start empty, otherwise the \
         post-call assertion cannot tell a write from a pre-existing value"
    );

    set_default(&harness, 1, "discord").await;

    assert_eq!(
        live_channel.read().expect("read live handle").clone(),
        Some("discord".to_string()),
        "channels_set_default persisted the choice but did not update the live proactive \
         routing handle, so proactive messages keep going to the old channel until the \
         process restarts (#3712)"
    );
}

fn channel_payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    let result = crate::rpc_harness::ok(value, context);
    result
        .get("data")
        .or_else(|| result.get("result"))
        .unwrap_or(result)
}

fn find_status_entry<'a>(entries: &'a [Value], channel: &str, auth_mode: &str) -> &'a Value {
    entries
        .iter()
        .find(|entry| {
            entry.get("channel_id").and_then(Value::as_str) == Some(channel)
                && entry.get("auth_mode").and_then(Value::as_str) == Some(auth_mode)
        })
        .unwrap_or_else(|| panic!("missing status entry for {channel}/{auth_mode}: {entries:?}"))
}

#[tokio::test]
async fn channels_imessage_config_only_connection_reports_status_and_disconnects() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let described = rpc(
        &harness.rpc_base,
        1,
        "openhuman.channels_describe",
        json!({ "channel": "imessage" }),
    )
    .await;
    assert_eq!(
        channel_payload(&described, "channels_describe")
            .get("id")
            .and_then(Value::as_str),
        Some("imessage")
    );

    let baseline = rpc(
        &harness.rpc_base,
        2,
        "openhuman.channels_status",
        json!({ "channel": "imessage" }),
    )
    .await;
    let baseline_entries = channel_payload(&baseline, "channels_status baseline")
        .as_array()
        .expect("status entries");
    assert_eq!(
        find_status_entry(baseline_entries, "imessage", "managed_dm")
            .get("connected")
            .and_then(Value::as_bool),
        Some(false)
    );

    let connected = rpc(
        &harness.rpc_base,
        3,
        "openhuman.channels_connect",
        json!({
            "channel": "imessage",
            "authMode": "managed_dm",
            "credentials": { "allowed_contacts": "alice@example.com, +15550100" }
        }),
    )
    .await;
    assert_eq!(
        channel_payload(&connected, "channels_connect imessage")
            .get("status")
            .and_then(Value::as_str),
        Some("connected")
    );

    let after_connect = rpc(
        &harness.rpc_base,
        4,
        "openhuman.channels_status",
        json!({ "channel": "imessage" }),
    )
    .await;
    let connected_entries = channel_payload(&after_connect, "channels_status after connect")
        .as_array()
        .expect("status entries");
    assert_eq!(
        find_status_entry(connected_entries, "imessage", "managed_dm")
            .get("connected")
            .and_then(Value::as_bool),
        Some(true),
        "config-only iMessage connection must be visible through channels_status"
    );

    let disconnected = rpc(
        &harness.rpc_base,
        5,
        "openhuman.channels_disconnect",
        json!({
            "channel": "imessage",
            "authMode": "managed_dm",
            "clearMemory": false
        }),
    )
    .await;
    assert_eq!(
        channel_payload(&disconnected, "channels_disconnect imessage")
            .get("disconnected")
            .and_then(Value::as_bool),
        Some(true)
    );

    let after_disconnect = rpc(
        &harness.rpc_base,
        6,
        "openhuman.channels_status",
        json!({ "channel": "imessage" }),
    )
    .await;
    let disconnected_entries =
        channel_payload(&after_disconnect, "channels_status after disconnect")
            .as_array()
            .expect("status entries");
    assert_eq!(
        find_status_entry(disconnected_entries, "imessage", "managed_dm")
            .get("connected")
            .and_then(Value::as_bool),
        Some(false)
    );
}
