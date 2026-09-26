use super::*;
use crate::agent::harness::with_current_sandbox_mode;
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
            tinytools::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Run `future` on a dedicated thread with a larger-than-default stack.
///
/// A `#[tokio::test]` future runs on the test harness's own OS thread,
/// whose default stack (2 MiB, same as a tokio worker thread) is marginal
/// for any test that reaches the real per-action Composio dispatch path
/// (`ComposioActionTool::execute` → `module_client::call` →
/// `crate::modules::connectors::call`): loading a native module that
/// isn't present in a unit-test binary runs a legitimately deep,
/// synchronous error-classification chain that can overflow it. Use this
/// for any test that calls `.execute()`/`.execute_unredacted()` on a path
/// the contract gate doesn't short-circuit before dispatch.
fn run_with_big_stack<F, Fut>(make_future: F) -> Fut::Output
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future,
    Fut::Output: Send + 'static,
{
    // Takes a closure that BUILDS the future, rather than the future
    // itself, so a `!Send` local held across an await point (e.g. a
    // `std::sync::MutexGuard`, as several of these tests hold via
    // `TEST_ENV_LOCK`) never has to cross the thread boundary — only the
    // closure does, and a single-threaded runtime's `block_on` does not
    // require `Future: Send`.
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build current-thread tokio runtime")
                .block_on(make_future())
        })
        .expect("spawn big-stack test thread")
        .join()
        .expect("big-stack test thread panicked")
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

#[test]
fn sandbox_read_only_blocks_per_action_write_call() {
    run_with_big_stack(|| async {
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
    });
}

#[test]
fn sandbox_read_only_blocks_per_action_admin_call() {
    run_with_big_stack(|| async {
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
    });
}

#[test]
fn sandbox_unset_leaves_per_action_execute_to_downstream() {
    run_with_big_stack(|| async {
        use crate::config::TEST_ENV_LOCK;
        let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    });
}

#[test]
fn contract_gate_surfaces_full_contract_then_proceeds_on_retry() {
    run_with_big_stack(|| async {
        // The FIRST per-action execute this turn must return the action's FULL
        // live contract (so the model composes a well-formed query) instead of
        // the thin spawn-time schema; the retry then proceeds to real dispatch.
        // A unique toolkit is seeded so this is deterministic and never touches
        // the network.
        use crate::config::TEST_ENV_LOCK;
        use crate::integrations::composio::catalog::{seed_live_catalog_cache, ToolContract};
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
    });
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
    let msg = match crate::integrations::composio::client::create_composio_client(&config) {
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
    config.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());

    // Direct mode + an api key must resolve to the Direct variant —
    // never the backend branch. (Deterministic: pure factory call, no
    // env / reload / await; see the note on the backend test.)
    let kind = crate::integrations::composio::client::create_composio_client(&config)
        .expect("direct mode with an api key must resolve");
    assert!(
        matches!(
            kind,
            crate::integrations::composio::client::ComposioClientKind::Direct(_)
        ),
        "direct-mode config must route to the Direct client, not backend"
    );
}

#[test]
fn mode_toggle_between_calls_is_observed() {
    run_with_big_stack(|| async {
        // Building the tool once with one mode and toggling the config
        // mid-session must take effect on the next execute. We can't trivially
        // mutate an `Arc<Config>`
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
        use crate::config::TEST_ENV_LOCK;
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
        direct_config.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.to_string();
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
    });
}

// `execute_unredacted` returns the config it resolved alongside the
// outcome, and `execute` redacts against that returned config instead of
// `self.config`. Both tests below seed a contract-gate catalog entry (same
// technique as `contract_gate_surfaces_full_contract_then_proceeds_on_retry`
// above) so the FIRST call surfaces the contract and returns right after
// resolving `live_config`, never reaching real dispatch.

#[test]
fn deferred_instance_returns_live_config_for_redaction() {
    run_with_big_stack(|| async {
        use crate::config::TEST_ENV_LOCK;
        use crate::integrations::composio::catalog::{seed_live_catalog_cache, ToolContract};
        let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _serialised = super::super::module_client::module_guard().await;

        let toolkit = "deferredredact";
        let slug = "DEFERREDREDACT_FETCH_ITEMS";
        seed_live_catalog_cache(
            toolkit,
            vec![ToolContract {
                slug: slug.to_string(),
                toolkit: toolkit.to_string(),
                description: Some("fetch items".to_string()),
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
        config.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.to_string();
        config.composio.api_key = Some("test-direct-key-for-deferred-redaction".to_string());
        config.save().await.expect("save config to disk");

        // Deferred instances (the parent-session catalogue synthesised beside
        // delegation tools) have no spawn-time config to anchor to — `config`
        // is `None` — so before this fix `execute()`'s `None => outcome` arm
        // returned every deferred outcome completely unredacted.
        let t = ComposioActionTool::deferred(toolkit, slug.to_string(), "fetch".to_string(), None);
        assert!(
            t.config.is_none(),
            "deferred instances carry no spawn-time config"
        );

        // Boxed exactly like `Tool::execute` does.
        let (live_config, outcome) = Box::pin(t.execute_unredacted(serde_json::json!({}))).await;
        let first = outcome.expect("contract-gate surface is Ok(error result)");
        assert!(
            first.is_error && error_text(&first).contains("Input JSON schema"),
            "first call must surface the contract, not reach real dispatch: {}",
            error_text(&first)
        );
        assert_eq!(
            live_config.composio.api_key.as_deref(),
            Some("test-direct-key-for-deferred-redaction"),
            "a deferred instance must return the live config actually used for this call, \
         not a default placeholder — otherwise execute() has nothing to redact against"
        );
    });
}

#[test]
fn configured_instance_resolves_live_config_not_the_stale_snapshot() {
    run_with_big_stack(|| async {
        // `execute_unredacted` assigns its `live_config` local from exactly
        // this private helper (`self.live_config().await`) before ever touching
        // the contract gate or dispatch. Calling it directly exercises the
        // `Some(snapshot)` branch, which reloads from `config_path` rather than
        // trusting the captured snapshot.
        use crate::config::TEST_ENV_LOCK;
        let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let tmp = tempfile::tempdir().expect("tempdir");

        // The on-disk config is the source of truth `live_composio_config`
        // reloads from; it holds the CURRENT (rotated) key.
        let mut on_disk = Config::default();
        on_disk.config_path = tmp.path().join("config.toml");
        on_disk.workspace_dir = tmp.path().join("workspace");
        on_disk.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.to_string();
        on_disk.composio.api_key = Some("rotated-key-after-registration".to_string());
        on_disk.save().await.expect("save config to disk");

        // The tool's own spawn-time snapshot is stale — it still carries the
        // key that was active when the sub-agent was created, before a
        // mid-session credential rotation.
        let mut stale_snapshot = on_disk.clone();
        stale_snapshot.composio.api_key = Some("stale-key-from-registration".to_string());
        drop(on_disk);

        let t = ComposioActionTool::new(
            Arc::new(stale_snapshot),
            "GMAIL_FETCH_EMAILS".to_string(),
            "fetch".to_string(),
            None,
        );

        let live_config = Box::pin(t.live_config())
            .await
            .expect("reload from a valid on-disk config must succeed");
        assert_eq!(
            live_config.composio.api_key.as_deref(),
            Some("rotated-key-after-registration"),
            "a configured instance must resolve the live (reloaded) config, not the stale \
         spawn-time snapshot — execute_unredacted feeds this exact value to both dispatch \
         and, after this fix, redaction"
        );
    });
}
