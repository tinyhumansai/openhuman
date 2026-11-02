use super::*;
use crate::core::runtime::DispatchCtx;

#[tokio::test]
async fn get_config_snapshot_wraps_snapshot_in_rpc_outcome() {
    let tmp = tempdir().unwrap();
    let cfg = tmp_config(&tmp);
    let outcome = get_config_snapshot(&cfg).await.expect("snapshot");
    assert!(outcome.value.get("config").is_some());
    assert!(outcome
        .logs
        .iter()
        .any(|l| l.contains("config loaded from")));
}

#[tokio::test]
async fn get_onboarding_completed_reads_from_loaded_config() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let outcome = get_onboarding_completed(&DispatchCtx::process_default())
        .await
        .expect("ok");
    // Default value — either true or false is fine; we just verify the call path.
    let _ = outcome.value;
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn load_and_resolve_api_url_returns_api_url_in_response() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let outcome = load_and_resolve_api_url(&DispatchCtx::process_default())
        .await
        .expect("ok");
    assert!(outcome.value.get("api_url").is_some());
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[test]
fn resolve_api_url_is_the_control_plane_the_host_resolved() {
    // The override guard itself lives with the host's enricher
    // (`openhuman-tinyhumans`); the core only forwards the resolved value, and
    // reports none when no host resolved a control plane.
    let mut config = Config::default();
    config.api_url = Some("http://127.0.0.1:4010".to_string());
    assert_eq!(
        resolve_backend_api_url(&config),
        None,
        "a configured api_url alone is not a resolved control plane"
    );
    config.resolved_endpoints.control_plane_url = Some("http://127.0.0.1:4010".to_string());
    assert_eq!(
        resolve_backend_api_url(&config).as_deref(),
        Some("http://127.0.0.1:4010")
    );
}

#[tokio::test]
async fn workspace_onboarding_flag_resolve_rejects_invalid_and_defaults() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let err = workspace_onboarding_flag_resolve(
        &DispatchCtx::process_default(),
        Some("a/b".into()),
        "done",
    )
    .await
    .unwrap_err();
    assert!(err.contains("Invalid onboarding flag"));

    // Happy path: default name on a fresh workspace → file doesn't exist.
    let outcome =
        workspace_onboarding_flag_resolve(&DispatchCtx::process_default(), None, "onboarding.done")
            .await
            .expect("ok");
    let _ = outcome.value;
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn workspace_onboarding_flag_set_rejects_invalid_names() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    for bad in ["", "   ", "a/b", "a\\b", ".."] {
        let err = workspace_onboarding_flag_set(
            &DispatchCtx::process_default(),
            Some(bad.into()),
            "default",
            true,
        )
        .await
        .unwrap_err();
        assert!(err.contains("Invalid onboarding flag"), "name {bad}: {err}");
    }
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn workspace_onboarding_flag_set_round_trip() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    // Create flag
    let created = workspace_onboarding_flag_set(
        &DispatchCtx::process_default(),
        Some("onboarding.done".into()),
        "default",
        true,
    )
    .await
    .expect("create");
    assert!(created.value);
    // Remove flag
    let removed = workspace_onboarding_flag_set(
        &DispatchCtx::process_default(),
        Some("onboarding.done".into()),
        "default",
        false,
    )
    .await
    .expect("remove");
    assert!(!removed.value);
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn apply_model_settings_trims_and_clears_optional_provider_fields() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    let set = ModelSettingsPatch {
        inference_url: Some(" https://llm.example.test/v1 ".into()),
        primary_cloud: Some(" provider-a ".into()),
        reasoning_provider: Some(" provider-reasoning ".into()),
        agentic_provider: Some(" provider-agentic ".into()),
        coding_provider: Some(" provider-coding ".into()),
        vision_provider: Some(" provider-vision ".into()),
        memory_provider: Some(" provider-memory ".into()),
        embeddings_provider: Some(" provider-embed ".into()),
        ..Default::default()
    };
    apply_model_settings(&mut cfg, set)
        .await
        .expect("set providers");
    assert_eq!(
        cfg.inference_url.as_deref(),
        Some("https://llm.example.test/v1")
    );
    assert_eq!(cfg.primary_cloud.as_deref(), Some("provider-a"));
    assert_eq!(
        cfg.reasoning_provider.as_deref(),
        Some("provider-reasoning")
    );
    assert_eq!(cfg.vision_provider.as_deref(), Some("provider-vision"));

    let clear = ModelSettingsPatch {
        inference_url: Some("   ".into()),
        primary_cloud: Some("".into()),
        reasoning_provider: Some(" ".into()),
        agentic_provider: Some(" ".into()),
        coding_provider: Some(" ".into()),
        vision_provider: Some(" ".into()),
        memory_provider: Some(" ".into()),
        embeddings_provider: Some(" ".into()),
        ..Default::default()
    };
    apply_model_settings(&mut cfg, clear)
        .await
        .expect("clear providers");
    assert!(cfg.inference_url.is_none());
    assert!(cfg.primary_cloud.is_none());
    assert!(cfg.reasoning_provider.is_none());
    assert!(cfg.agentic_provider.is_none());
    assert!(cfg.coding_provider.is_none());
    assert!(cfg.vision_provider.is_none());
    assert!(cfg.memory_provider.is_none());
    assert!(cfg.embeddings_provider.is_none());
}

// ── apply_autonomy_settings ────────────────────────────────────

#[tokio::test]
async fn apply_autonomy_settings_persists_max_actions_per_hour() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let outcome = apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch {
            max_actions_per_hour: Some(200),
            ..Default::default()
        },
    )
    .await
    .expect("apply");
    assert_eq!(cfg.autonomy.max_actions_per_hour, 200);
    // Snapshot returned so the caller can echo the saved state.
    assert!(outcome.value.get("config").is_some());
    // Round-trip from disk: reload the saved TOML and confirm.
    let on_disk = tokio::fs::read_to_string(&cfg.config_path).await.unwrap();
    assert!(
        on_disk.contains("max_actions_per_hour = 200"),
        "expected TOML to contain max_actions_per_hour = 200, got:\n{on_disk}"
    );
}

#[tokio::test]
async fn apply_autonomy_settings_no_op_when_patch_empty() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let prior = cfg.autonomy.max_actions_per_hour;
    let _ = apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch {
            max_actions_per_hour: None,
            ..Default::default()
        },
    )
    .await
    .expect("apply noop");
    assert_eq!(cfg.autonomy.max_actions_per_hour, prior);
}

#[tokio::test]
async fn apply_autonomy_settings_rejects_zero() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let err = apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch {
            max_actions_per_hour: Some(0),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("at least 1"),
        "expected validation error, got: {err}"
    );
}

#[tokio::test]
async fn apply_autonomy_settings_accepts_unlimited_sentinel() {
    let _g = ENV_LOCK.lock().await;
    // u32::MAX is the new "unlimited" sentinel exposed by the UI as a
    // preset. The upper cap was lifted in the same PR that defaulted
    // fresh installs to u32::MAX; anything in [1, u32::MAX] should now
    // round-trip cleanly.
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch {
            max_actions_per_hour: Some(u32::MAX),
            ..Default::default()
        },
    )
    .await
    .expect("u32::MAX (unlimited) should round-trip");
    assert_eq!(cfg.autonomy.max_actions_per_hour, u32::MAX);
}

#[tokio::test]
async fn load_and_apply_autonomy_settings_roundtrip() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

    let patch = AutonomySettingsPatch {
        max_actions_per_hour: Some(500),
        ..Default::default()
    };
    let outcome = load_and_apply_autonomy_settings(&DispatchCtx::process_default(), patch)
        .await
        .expect("apply");
    assert!(outcome.value.get("config").is_some());

    // Reload from scratch and confirm the saved value sticks.
    let reloaded = load_config_with_timeout().await.expect("reload");
    assert_eq!(reloaded.autonomy.max_actions_per_hour, 500);

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn apply_autonomy_settings_replaces_auto_approve() {
    // ENV_LOCK serializes the `live_policy::reload_from` triggered by
    // `apply_autonomy_settings` against other live-policy-touching tests.
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch {
            auto_approve: Some(vec!["shell".into(), "curl".into()]),
            ..Default::default()
        },
    )
    .await
    .expect("apply auto_approve");
    assert_eq!(cfg.autonomy.auto_approve, vec!["shell", "curl"]);
    // Persisted to the TOML, not just held in memory.
    let on_disk = tokio::fs::read_to_string(&cfg.config_path).await.unwrap();
    assert!(
        on_disk.contains("auto_approve") && on_disk.contains("shell") && on_disk.contains("curl"),
        "auto_approve allowlist should round-trip to TOML, got:\n{on_disk}"
    );
}

#[tokio::test]
async fn autonomy_auto_approve_all_defaults_false() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let cfg = tmp_config(&tmp);
    assert!(
        !cfg.autonomy.auto_approve_all,
        "fresh AutonomyConfig must default auto_approve_all to false"
    );
}

#[tokio::test]
async fn autonomy_auto_approve_all_persists() {
    // ENV_LOCK serializes the `live_policy::reload_from` triggered by
    // `apply_autonomy_settings` against other live-policy-touching tests.
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch {
            auto_approve_all: Some(true),
            ..Default::default()
        },
    )
    .await
    .expect("apply auto_approve_all=true");
    assert!(cfg.autonomy.auto_approve_all);
    let on_disk = tokio::fs::read_to_string(&cfg.config_path).await.unwrap();
    assert!(
        on_disk.contains("auto_approve_all = true"),
        "expected TOML to persist auto_approve_all = true, got:\n{on_disk}"
    );

    // Parse the saved TOML directly (rather than `load_config_with_timeout`,
    // which resolves the workspace from `OPENHUMAN_WORKSPACE`/discovery and
    // `tmp_config` doesn't point that at `tmp`) to confirm the value survives
    // a fresh deserialize, then flip it back off and confirm that round-trips
    // too.
    let on_disk_cfg: crate::config::Config = toml::from_str(&on_disk).expect("parse saved TOML");
    assert!(on_disk_cfg.autonomy.auto_approve_all);

    apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch {
            auto_approve_all: Some(false),
            ..Default::default()
        },
    )
    .await
    .expect("apply auto_approve_all=false");
    assert!(!cfg.autonomy.auto_approve_all);
    let on_disk_after = tokio::fs::read_to_string(&cfg.config_path).await.unwrap();
    assert!(
        on_disk_after.contains("auto_approve_all = false"),
        "expected TOML to persist auto_approve_all = false, got:\n{on_disk_after}"
    );
}

#[tokio::test]
async fn add_auto_approve_tool_appends_then_dedupes() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

    add_auto_approve_tool(&DispatchCtx::process_default(), "git_operations")
        .await
        .expect("first add");
    // Idempotent: a second add of the same tool must not create a duplicate.
    add_auto_approve_tool(&DispatchCtx::process_default(), "git_operations")
        .await
        .expect("second add (idempotent)");

    let reloaded = load_config_with_timeout().await.expect("reload");
    let hits = reloaded
        .autonomy
        .auto_approve
        .iter()
        .filter(|t| t.as_str() == "git_operations")
        .count();
    assert_eq!(
        hits, 1,
        "tool must appear exactly once after duplicate adds"
    );

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

// ── agent settings (action/tool timeout, issue #3100) ───────────────────────

#[tokio::test]
async fn apply_agent_settings_updates_timeout_and_persists_snapshot() {
    // ENV_LOCK: `set_tool_timeout_secs` reads OPENHUMAN_TOOL_TIMEOUT_SECS and
    // mutates the process-global timeout; serialize against other env-touching
    // tests and ensure no operator override is masking the config value.
    let _g = ENV_LOCK.lock().await;
    unsafe {
        std::env::remove_var("OPENHUMAN_TOOL_TIMEOUT_SECS");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    let outcome = apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            agent_timeout_secs: Some(300),
            ..AgentSettingsPatch::default()
        },
    )
    .await
    .expect("apply agent settings");

    assert_eq!(cfg.agent.agent_timeout_secs, 300);
    assert_eq!(
        outcome.value["config"]["agent"]["agent_timeout_secs"],
        serde_json::json!(300)
    );
    assert!(outcome
        .logs
        .iter()
        .any(|l| l.contains("agent settings saved to")));
    // With no env override, the live runtime now reflects the saved value.
    assert_eq!(crate::tools::timeout::tool_execution_timeout_secs(), 300);
}

#[tokio::test]
async fn apply_autonomy_settings_rejects_a_relative_or_empty_trusted_root() {
    let _g = ENV_LOCK.lock().await;
    for root in ["", "  ", ".", "rel/dir"] {
        let tmp = tempdir().unwrap();
        let mut cfg = tmp_config(&tmp);
        let enabled_before = cfg.autonomy.enabled;
        let err = apply_autonomy_settings(
            &mut cfg,
            AutonomySettingsPatch {
                // A rejected patch must not half-apply its other fields.
                enabled: Some(!enabled_before),
                trusted_roots: Some(vec![crate::security::TrustedRoot {
                    path: root.into(),
                    access: crate::security::TrustedAccess::ReadWrite,
                }]),
                ..Default::default()
            },
        )
        .await
        .expect_err("a non-absolute trusted root must be refused");
        assert!(
            err.contains("absolute"),
            "expected validation error, got: {err}"
        );
        assert!(cfg.autonomy.trusted_roots.is_empty());
        assert_eq!(cfg.autonomy.enabled, enabled_before, "patch half-applied");
    }
}
