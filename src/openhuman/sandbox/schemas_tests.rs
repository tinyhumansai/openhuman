use super::*;
use crate::core::runtime::context::CoreContext;
use crate::core::runtime::DomainSet;

/// Scope a `Config` onto the embedder-config seam that `load_config_with_timeout()`
/// prefers, so a handler reads it instead of doing live disk I/O against
/// `~/.openhuman` — which is slow and racy across parallel unit tests (#6081).
fn scoped_ctx(config: crate::openhuman::config::Config) -> std::sync::Arc<CoreContext> {
    CoreContext::for_test_with_config(DomainSet::full(), config)
}

#[test]
fn all_schemas_are_in_sandbox_namespace() {
    for schema in all_controller_schemas() {
        assert_eq!(schema.namespace, "sandbox");
    }
}

#[test]
fn registered_controllers_match_schemas() {
    let schemas = all_controller_schemas();
    let controllers = all_registered_controllers();
    assert_eq!(schemas.len(), controllers.len());
    for (s, c) in schemas.iter().zip(controllers.iter()) {
        assert_eq!(s.function, c.schema.function);
    }
}

#[tokio::test]
async fn handle_status_returns_json() {
    let ctx = scoped_ctx(crate::openhuman::config::Config::default());
    let result = CoreContext::scope(ctx, async { handle_status(Map::new()).await }).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn handle_resolve_policy_none() {
    let mut params = Map::new();
    params.insert("sandbox_mode".into(), Value::String("none".into()));
    let ctx = scoped_ctx(crate::openhuman::config::Config::default());
    let result = CoreContext::scope(ctx, async { handle_resolve_policy(params).await }).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn handle_resolve_policy_sandboxed_remote() {
    let mut params = Map::new();
    params.insert("sandbox_mode".into(), Value::String("sandboxed".into()));
    params.insert("is_remote".into(), Value::Bool(true));
    let ctx = scoped_ctx(crate::openhuman::config::Config::default());
    let result = CoreContext::scope(ctx, async { handle_resolve_policy(params).await }).await;
    assert!(result.is_ok());
    let val = result.unwrap();
    let backend = val.get("backend").and_then(|b| b.as_str());
    assert_eq!(backend, Some("docker"));
}

/// Build a `Config` carrying a Docker runtime with a NON-default image, so a
/// test can prove the RPC handlers read the loaded runtime rather than
/// `RuntimeConfig::default()` (#6081).
fn docker_runtime_config() -> crate::openhuman::config::Config {
    let mut config = crate::openhuman::config::Config::default();
    config.workspace_dir = std::path::PathBuf::from("/tmp/openhuman-6081-ws");
    config.runtime.kind = "docker".into();
    config.runtime.docker.image = "ghcr.io/example/custom-sandbox:6081".into();
    config
}

/// #6081 regression — `sandbox_resolve_policy` must honor the loaded
/// `[runtime] kind = "docker"` config. Before the fix the handler answered from
/// `RuntimeConfig::default()` (`kind = "native"`), so a Docker-configured user
/// was told `backend = "local"`. The config is injected through the
/// embedder-config seam that `load_config_with_timeout()` reads, scoped for the
/// duration of the dispatch.
#[tokio::test]
async fn handle_resolve_policy_honors_loaded_docker_runtime() {
    let ctx = scoped_ctx(docker_runtime_config());

    let mut params = Map::new();
    params.insert("sandbox_mode".into(), Value::String("sandboxed".into()));
    params.insert("is_remote".into(), Value::Bool(false));

    let val = CoreContext::scope(ctx, async { handle_resolve_policy(params).await })
        .await
        .expect("resolve_policy should succeed under the scoped config");

    // Consequence #1: the backend follows `[runtime] kind`, not the default.
    assert_eq!(
        val.get("backend").and_then(|b| b.as_str()),
        Some("docker"),
        "docker runtime must resolve to the docker backend; got {val:?}"
    );

    // Consequence #2: docker overrides reflect the user's `[runtime.docker]`,
    // not the compiled-in default image.
    let image = val
        .get("docker_overrides")
        .and_then(|o| o.get("image"))
        .and_then(|i| i.as_str());
    assert_eq!(
        image,
        Some("ghcr.io/example/custom-sandbox:6081"),
        "docker_overrides.image must reflect the loaded [runtime.docker].image; got {val:?}"
    );
}

/// #6081 regression — `sandbox_status` must likewise read the loaded runtime.
/// With `[runtime] kind = "docker"` and `is_remote = false`, a sandboxed status
/// probe resolves the docker backend; before the fix it answered "local" from
/// `RuntimeConfig::default()`. Uses `backend = "local"` in the request to prove
/// the resolution comes from the loaded config's `kind`, not the caller's
/// backend name.
#[tokio::test]
async fn handle_status_honors_loaded_docker_runtime() {
    let ctx = scoped_ctx(docker_runtime_config());

    let mut params = Map::new();
    params.insert("backend".into(), Value::String("local".into()));
    params.insert("is_remote".into(), Value::Bool(false));

    let val = CoreContext::scope(ctx, async { handle_status(params).await })
        .await
        .expect("status should succeed under the scoped config");

    assert_eq!(
        val.get("kind").and_then(|k| k.as_str()),
        Some("docker"),
        "docker runtime must make status report the docker backend; got {val:?}"
    );
}

/// #6081 (F2) — `sandbox_resolve_policy` must use the loaded config's
/// already-resolved `action_dir`, not recompute it from `action_dir_override`
/// alone. An embedder sets `Config.action_dir` directly via
/// `CoreBuilder::action_dir(..)` and leaves `action_dir_override` at `None`;
/// `resolve_action_dir(&None)` would have ignored that and returned the default
/// projects dir, so the resolved policy would have pointed the sandbox at the
/// wrong root. This pins that the handler honors the embedder's programmatic
/// `action_dir`.
#[tokio::test]
async fn handle_resolve_policy_uses_loaded_action_dir() {
    let embedder_action_dir = std::path::PathBuf::from("/tmp/openhuman-6081-action-dir");
    let mut config = crate::openhuman::config::Config::default();
    // The embedder-shaped case: `action_dir` set directly, override untouched.
    config.action_dir = embedder_action_dir.clone();
    config.action_dir_override = None;
    let ctx = scoped_ctx(config);

    let mut params = Map::new();
    params.insert("sandbox_mode".into(), Value::String("none".into()));

    let val = CoreContext::scope(ctx, async { handle_resolve_policy(params).await })
        .await
        .expect("resolve_policy should succeed under the scoped config");

    assert_eq!(
        val.get("workspace_root").and_then(|w| w.as_str()),
        Some(embedder_action_dir.to_str().unwrap()),
        "policy workspace_root must use the loaded config.action_dir, not a \
         recomputed default; got {val:?}"
    );
}

#[tokio::test]
async fn handle_validate_policy_valid() {
    let policy = super::super::types::SandboxPolicy {
        backend: super::super::types::SandboxBackendKind::Docker,
        workspace_root: std::path::PathBuf::from("/tmp/safe"),
        read_only_mounts: vec![],
        allow_network: false,
        env_passthrough: vec![],
        docker_overrides: None,
    };
    let mut params = Map::new();
    params.insert("policy".into(), serde_json::to_value(&policy).unwrap());
    let result = handle_validate_policy(params).await;
    assert!(result.is_ok());
    let val = result.unwrap();
    assert_eq!(val.get("valid").and_then(|v| v.as_bool()), Some(true));
}

#[tokio::test]
async fn handle_validate_policy_dangerous() {
    let policy = super::super::types::SandboxPolicy {
        backend: super::super::types::SandboxBackendKind::Docker,
        workspace_root: std::path::PathBuf::from("/"),
        read_only_mounts: vec![],
        allow_network: false,
        env_passthrough: vec![],
        docker_overrides: Some(super::super::types::DockerOverrides {
            network: Some("host".into()),
            ..Default::default()
        }),
    };
    let mut params = Map::new();
    params.insert("policy".into(), serde_json::to_value(&policy).unwrap());
    let result = handle_validate_policy(params).await;
    assert!(result.is_ok());
    let val = result.unwrap();
    assert_eq!(val.get("valid").and_then(|v| v.as_bool()), Some(false));
}
