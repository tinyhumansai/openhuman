//! The module configuration follows `[computer]` and the stored credentials.

use super::*;
use crate::credentials::{api_key, AuthService};

fn config_in(dir: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.config_path = dir.join("config.toml");
    config.workspace_dir = dir.join("workspace");
    config.secrets.encrypt = false;
    config
}

async fn store(config: &Config, slug: &str, key: &str) {
    AuthService::from_config(config)
        .store_provider_token(
            &format!("provider:{slug}"),
            "default",
            key,
            Default::default(),
            true,
        )
        .await
        .expect("store provider key");
}

#[tokio::test]
async fn signed_in_host_routes_jev_and_planner_through_tinyhumans() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.resolved_endpoints.product_identity = Some("openhuman-test".into());
    config.computer.rescue_model = Some("openai/gpt-6-luna-pro".into());
    config.computer.planner_model = Some("anthropic/claude-sonnet-5".into());
    api_key::store_api_key(&config, "th_test_backend")
        .await
        .unwrap();
    let value = module_config(&config).await;
    assert_eq!(value["jev"]["provider"], "tiny_humans_open_router");
    assert_eq!(value["planner"]["provider"], "tiny_humans");
    assert_eq!(value["planner"]["api_key"], "th_test_backend");
    assert!(value["planner"]["sdk_name"].is_string());
    assert_eq!(value["planner"]["rescue_model"], "openai/gpt-6-luna-pro");
    assert_eq!(value["planner"]["model"], "anthropic/claude-sonnet-5");
    // The output shaper has no setting; the gateway runs it on its own model.
    assert_eq!(value["planner"]["output_model"], HOSTED_REASONING_MODEL);
    assert_eq!(billing_route(&config).await, "hosted");
}

#[tokio::test]
async fn signed_in_host_runs_unchosen_roles_on_a_model_the_gateway_serves() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    api_key::store_api_key(&config, "th_test_backend")
        .await
        .unwrap();
    // A blank choice is no choice.
    config.computer.rescue_model = Some("  ".into());
    let value = module_config(&config).await;
    for role in ["model", "rescue_model", "output_model"] {
        assert_eq!(value["planner"][role], HOSTED_REASONING_MODEL, "{role}");
    }
}

#[tokio::test]
async fn byok_host_uses_openrouter_for_both() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    store(&config, "openrouter", "or_test_direct").await;
    let value = module_config(&config).await;
    assert_eq!(value["jev"]["provider"], "open_router");
    assert_eq!(value["planner"]["provider"], "open_router");
    assert_eq!(value["planner"]["api_key"], "or_test_direct");
    // OpenRouter serves the module's own defaults, so none is sent.
    for role in ["model", "rescue_model", "output_model"] {
        assert!(value["planner"].get(role).is_none(), "{role}");
    }
    assert_eq!(billing_route(&config).await, "direct_openrouter");
}

#[tokio::test]
async fn open_jev_and_sage_use_their_own_keys() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    store(&config, "openjev", "oj_test").await;
    store(&config, "sage", "sage_test").await;

    config.computer.decision_model = DecisionModel::OpenJev;
    let value = module_config(&config).await;
    assert_eq!(
        value["jev"],
        json!({"api_key": "oj_test", "provider": "open_jev"})
    );
    assert_eq!(billing_route(&config).await, "open_jev");

    config.computer.decision_model = DecisionModel::Sage;
    config.computer.sage_fast = true;
    let value = module_config(&config).await;
    assert_eq!(
        value["jev"],
        json!({"api_key": "sage_test", "provider": "sage", "fast": true})
    );
    assert_eq!(billing_route(&config).await, "sage");
}

#[test]
fn tracing_is_off_unless_the_config_or_the_environment_turns_it_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    assert!(!tracing_enabled_with(&config, None));
    assert!(!tracing_enabled_with(&config, Some("0")));
    assert!(tracing_enabled_with(&config, Some("1")));
    assert!(tracing_enabled_with(&config, Some(" true ")));
    config.computer.trace = true;
    assert!(tracing_enabled_with(&config, None));
}

#[tokio::test]
async fn a_traced_host_gives_the_module_a_desktop_trace_path_in_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.computer.trace = true;
    let value = module_config(&config).await;
    let expected = trace_dir(&config).join("desktop-trace.jsonl");
    assert_eq!(value["trace_path"], json!(expected.to_string_lossy()));
    assert!(
        trace_dir(&config).is_dir(),
        "the trace directory is created"
    );
    assert!(expected.starts_with(&config.workspace_dir));
}

#[tokio::test]
async fn chrome_path_is_the_module_browser_executable() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.browser.chrome_path = Some("/Applications/Chrome.app".into());
    let value = module_config(&config).await;
    assert_eq!(
        value["browser"],
        json!({"executable": "/Applications/Chrome.app"})
    );
}

#[tokio::test]
async fn a_trace_directory_that_cannot_be_made_still_leaves_the_module_configured() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.computer.trace = true;
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    // `state` is a file, so the trace directory cannot be made.
    std::fs::write(config.workspace_dir.join("state"), b"not a directory").unwrap();
    let value = module_config(&config).await;
    assert!(value["trace_path"].is_string());
    assert!(!trace_dir(&config).is_dir());
}

#[cfg(unix)]
#[tokio::test]
async fn the_trace_directory_is_readable_by_its_owner_alone() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.computer.trace = true;
    // A directory left open by an earlier run is narrowed too.
    std::fs::create_dir_all(trace_dir(&config)).unwrap();
    std::fs::set_permissions(trace_dir(&config), std::fs::Permissions::from_mode(0o755)).unwrap();
    let _ = module_config(&config).await;
    let mode = std::fs::metadata(trace_dir(&config))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
}
