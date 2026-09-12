use super::*;
use crate::openhuman::agent::harness::with_current_sandbox_mode;
use std::path::Path;

struct WorkspaceEnvGuard {
    previous: Option<std::ffi::OsString>,
}

impl WorkspaceEnvGuard {
    fn set(path: &Path) -> Self {
        let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
        Self::set_current(path);
        Self { previous }
    }

    fn set_current(path: &Path) {
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
        }
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        unsafe {
            match self.previous.take() {
                Some(value) => std::env::set_var("OPENHUMAN_WORKSPACE", value),
                None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
            }
        }
    }
}

/// Build a minimal `Arc<Config>` with `composio.mode = "backend"`
/// (the default). The sandbox gate runs *before* any HTTP call or
/// factory resolve, so these tests never reach the network. Mirrors
/// the helper in `tools_tests.rs`.
fn fake_config() -> Arc<Config> {
    let tmp = tempfile::tempdir().expect("tempdir for fake_config");
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    // Leak the tempdir so the path remains valid for the test's
    // lifetime — `Config::config_path` is just used as a lookup key
    // here, not actually written to.
    std::mem::forget(tmp);
    Arc::new(config)
}

// Direct-mode coverage no longer constructs an `Arc<Config>` helper:
// `ComposioActionTool::execute` reloads config from the tool
// snapshot's `config_path` per call (#1710 Wave 4), so direct-mode
// tests persist an isolated `config.toml` and pass that config into
// the constructor.

fn error_text(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match c {
            crate::openhuman::tools::traits::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn humanize_composio_action_sentence_cases_slug() {
    assert_eq!(
        humanize_composio_action("GMAIL_SEND_EMAIL"),
        "Gmail send email"
    );
    assert_eq!(
        humanize_composio_action("GOOGLECALENDAR_EVENTS_LIST"),
        "Googlecalendar events list"
    );
    assert_eq!(humanize_composio_action(""), "");
}

#[test]
fn display_label_is_human_and_detail_pulls_recipient() {
    let tool = ComposioActionTool::new(
        fake_config(),
        "GMAIL_SEND_EMAIL".to_string(),
        "Send an email via Gmail".to_string(),
        None,
    );
    assert_eq!(
        tool.display_label(&serde_json::Value::Null).as_deref(),
        Some("Gmail send email")
    );
    assert_eq!(
        tool.display_detail(&serde_json::json!({ "recipient_email": "steven@gmail.com" }))
            .as_deref(),
        Some("steven@gmail.com")
    );
}

#[test]
fn per_action_tool_requires_approval_for_external_writes_only() {
    let write = ComposioActionTool::new(
        fake_config(),
        "GMAIL_SEND_EMAIL".to_string(),
        "send".to_string(),
        None,
    );
    let read = ComposioActionTool::new(
        fake_config(),
        "GMAIL_FETCH_EMAILS".to_string(),
        "fetch".to_string(),
        None,
    );

    assert!(write.external_effect_with_args(&serde_json::Value::Null));
    assert!(!read.external_effect_with_args(&serde_json::Value::Null));
}

#[tokio::test]
async fn sandbox_read_only_blocks_per_action_write_call() {
    let t = ComposioActionTool::new(
        fake_config(),
        "GMAIL_SEND_EMAIL".to_string(),
        "send a gmail message".to_string(),
        None,
    );
    let result = with_current_sandbox_mode(SandboxMode::ReadOnly, async {
        t.execute(serde_json::json!({})).await.unwrap()
    })
    .await;
    assert!(
        result.is_error,
        "per-action Write under read-only must error"
    );
    let msg = error_text(&result);
    assert!(msg.contains("strict read-only"), "got: {msg}");
    assert!(msg.contains("`write`"), "got: {msg}");
}

#[tokio::test]
async fn sandbox_read_only_blocks_per_action_admin_call() {
    let t = ComposioActionTool::new(
        fake_config(),
        "GMAIL_DELETE_EMAIL".to_string(),
        "destructive".to_string(),
        None,
    );
    let result = with_current_sandbox_mode(SandboxMode::ReadOnly, async {
        t.execute(serde_json::json!({})).await.unwrap()
    })
    .await;
    assert!(result.is_error);
    let msg = error_text(&result);
    assert!(msg.contains("`admin`"), "got: {msg}");
}

#[tokio::test]
async fn sandbox_unset_leaves_per_action_execute_to_downstream() {
    // Outside any `with_current_sandbox_mode` scope the task-local
    // is `None` and the gate is a no-op. The downstream factory
    // resolve still fails (no backend session token / no api key),
    // but never with the sandbox text.
    //
    // The sandbox gate is a no-op here, so dispatch falls through to
    // the live config reload (#1710 Wave 4). Hold `TEST_ENV_LOCK`
    // and point `OPENHUMAN_WORKSPACE` at an isolated, persisted
    // config for compatibility with sibling config-loading tests.
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // The module is one instance per process holding one route, and both
    // halves below reconfigure it. Without this they race any other test
    // that also points it somewhere.
    let _serialised = super::super::module_client::module_guard().await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let _workspace_guard = WorkspaceEnvGuard::set(tmp.path());

    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.save().await.expect("save fake config to disk");

    let t = ComposioActionTool::new(
        Arc::new(config),
        "GMAIL_SEND_EMAIL".to_string(),
        "send".to_string(),
        None,
    );
    let result = t.execute(serde_json::json!({})).await.unwrap();
    let msg = error_text(&result);
    assert!(
        !msg.contains("strict read-only"),
        "unset sandbox must never trigger the gate, got: {msg}"
    );
}

#[tokio::test]
async fn contract_gate_surfaces_full_contract_then_proceeds_on_retry() {
    // Regression for #4853: the FIRST per-action execute this turn must
    // return the action's FULL live contract (so the model composes a
    // well-formed query) instead of running with the thin spawn-time
    // schema; the retry then proceeds to real dispatch. A unique toolkit is
    // seeded so this is deterministic and never touches the network.
    use crate::openhuman::config::TEST_ENV_LOCK;
    use crate::openhuman::integrations::composio::catalog::{
        seed_live_catalog_cache, ToolContract,
    };
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let toolkit = "cgateexec";
    let slug = "CGATEEXEC_FETCH_ITEMS";
    seed_live_catalog_cache(
        toolkit,
        vec![ToolContract {
            slug: slug.to_string(),
            toolkit: toolkit.to_string(),
            description: Some("Search items. Quote multi-word phrases.".to_string()),
            required_args: vec!["query".to_string()],
            input_schema: Some(serde_json::json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            })),
            output_fields: Vec::new(),
            output_schema: None,
            primary_array_path: None,
            is_curated: false,
        }],
    );

    let tmp = tempfile::tempdir().expect("tempdir");
    let _workspace_guard = WorkspaceEnvGuard::set(tmp.path());
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.save().await.expect("save fake config to disk");

    let t = ComposioActionTool::new(
        Arc::new(config),
        slug.to_string(),
        "search items".to_string(),
        None,
    );

    // First call: gate surfaces the contract (recoverable tool error).
    let first = t.execute(serde_json::json!({})).await.unwrap();
    assert!(
        first.is_error,
        "first call must surface a recoverable error"
    );
    let first_msg = error_text(&first);
    assert!(
        first_msg.contains("Input JSON schema"),
        "first call must carry the full contract, got: {first_msg}"
    );

    // Retry: gate proceeds; dispatch fails downstream (no session token) but
    // crucially NOT with the contract text — proving the gate did not block.
    let second = t.execute(serde_json::json!({})).await.unwrap();
    let second_msg = error_text(&second);
    assert!(
        !second_msg.contains("Input JSON schema"),
        "retry must proceed past the gate to real dispatch, got: {second_msg}"
    );
}

// ── Factory routing (#1710) ──────────────────────────────────────
//
// Regression coverage for the bug fix: `ComposioActionTool` now
// resolves its client per call rather than caching one at
// construction time, so a mid-session `composio.mode` toggle is
// honoured on the very next per-action execute.

// These two tests assert the *factory routing decision* by mode. They
// call `create_composio_client(&Config)` directly — the pure routing
// function — instead of going through `tool.execute()`, which reloads
// config via `load_config_with_timeout()` (reads `OPENHUMAN_WORKSPACE`)
// and was therefore subject to a parallel-test env-var race: another
// non-`TEST_ENV_LOCK` test mutating `OPENHUMAN_WORKSPACE` in the await
// window flipped the reloaded config, intermittently failing
// `factory_routes_through_direct_when_mode_is_direct`. The factory reads
// mode + session purely from the passed `Config` (the auth-store path is
// derived from the config's own paths, not the env var), so pointing
// those at a fresh tempdir is fully isolated, deterministic, and needs
// no env mutation / `TEST_ENV_LOCK` / async.
#[test]
fn factory_routes_through_backend_when_mode_is_backend() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default(); // composio.mode defaults to "backend"
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");

    // `ComposioClientKind` isn't `Debug`, so match rather than
    // `expect_err` (which would need to format the unexpected `Ok`).
    let msg =
        match crate::openhuman::integrations::composio::client::create_composio_client(&config) {
            Ok(_) => panic!("backend mode with no session must error, but a client resolved"),
            Err(e) => e.to_string(),
        };
    assert!(
        msg.contains("backend") || msg.contains("session"),
        "expected backend-mode session error, got: {msg}"
    );
    assert!(
        !msg.contains("direct mode"),
        "backend-mode failure must not surface direct-mode artifacts: {msg}"
    );
}

#[test]
fn factory_routes_through_direct_when_mode_is_direct() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());

    // Direct mode + an api key must resolve to the Direct variant —
    // never the backend branch. (Deterministic: pure factory call, no
    // env / reload / await; see the note on the backend test.)
    let kind = crate::openhuman::integrations::composio::client::create_composio_client(&config)
        .expect("direct mode with an api key must resolve");
    assert!(
        matches!(
            kind,
            crate::openhuman::integrations::composio::client::ComposioClientKind::Direct(_)
        ),
        "direct-mode config must route to the Direct client, not backend"
    );
}

#[tokio::test]
async fn mode_toggle_between_calls_is_observed() {
    // Regression test for #1710: building the tool once with one
    // mode and toggling the config mid-session must take effect on
    // the next execute. We can't trivially mutate an `Arc<Config>`
    // without `Arc::get_mut` (single ref), so we run the two halves
    // sequentially against two different on-disk configs and assert
    // each routes through its respective branch. This captures the
    // core structural property — that no client is baked at
    // construction time — and is faithful to production because
    // `.execute(..)` reloads from the tool snapshot's `config_path`
    // per call.
    //
    // The actual in-place mutation flow on the live system is:
    // RPC `composio.set_mode` writes config.toml, the
    // `ComposioConfigChanged` event invalidates the parent
    // session's `Arc<Config>`, and the next sub-agent spawn picks
    // up the fresh `Arc<Config>` from
    // `Config::load_or_init().await`. Here we simulate that by
    // rewriting `OPENHUMAN_WORKSPACE/config.toml` between the two
    // halves while holding `TEST_ENV_LOCK`.
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // The module is one instance per process holding one route, and both
    // halves below reconfigure it. Without this they race any other test
    // that also points it somewhere.
    let _serialised = super::super::module_client::module_guard().await;

    // ── Backend half ────────────────────────────────────────────
    let tmp_backend = tempfile::tempdir().expect("tempdir backend");
    let _workspace_guard = WorkspaceEnvGuard::set(tmp_backend.path());
    let mut backend_config = Config::default();
    backend_config.config_path = tmp_backend.path().join("config.toml");
    backend_config.workspace_dir = tmp_backend.path().join("workspace");
    backend_config
        .save()
        .await
        .expect("save backend config to disk");

    let backend_tool = ComposioActionTool::new(
        Arc::new(backend_config),
        "GMAIL_FETCH_EMAILS".to_string(),
        "read-shaped slug".to_string(),
        None,
    );
    let backend_result = backend_tool.execute(serde_json::json!({})).await.unwrap();
    let backend_msg = error_text(&backend_result);
    // Backend mode with nothing signed in must *fail*, naming what is
    // missing. The wording moved into the connector module along with the
    // client — the host can no longer name a route, so the module reports
    // it holds none — so this asserts the contract rather than the phrase.
    assert!(
        backend_msg.contains("backend")
            || backend_msg.contains("session")
            || backend_msg.contains("route"),
        "backend-mode tool should say what is missing, got: {backend_msg}"
    );

    // ── Direct half ─────────────────────────────────────────────
    let tmp_direct = tempfile::tempdir().expect("tempdir direct");
    WorkspaceEnvGuard::set_current(tmp_direct.path());
    let mut direct_config = Config::default();
    direct_config.config_path = tmp_direct.path().join("config.toml");
    direct_config.workspace_dir = tmp_direct.path().join("workspace");
    direct_config.composio.mode =
        crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    direct_config.composio.api_key = Some("test-direct-key".to_string());
    direct_config
        .save()
        .await
        .expect("save direct config to disk");

    let direct_tool = ComposioActionTool::new(
        Arc::new(direct_config),
        "GMAIL_FETCH_EMAILS".to_string(),
        "read-shaped slug".to_string(),
        None,
    );
    let direct_result = direct_tool.execute(serde_json::json!({})).await.unwrap();
    let direct_msg = error_text(&direct_result);

    // Direct tool's error must NOT mention a backend session — the
    // smoking gun for the pre-fix bug would have been the
    // direct-mode tool surfacing
    // `staging-api.tinyhumans.ai` / `no backend session` because
    // the cached client was a backend handle.
    assert!(
        !direct_msg.contains("no backend session"),
        "direct-mode tool must not surface backend-session artifacts: {direct_msg}"
    );
}
