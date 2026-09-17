use super::*;
use tempfile::tempdir;

use crate::config::TEST_ENV_LOCK as ENV_LOCK;

fn tmp_config(tmp: &tempfile::TempDir) -> Config {
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().join("workspace");
    cfg.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
    cfg
}

// ── apply_autonomy_settings ────────────────────────────────────

#[tokio::test]
async fn apply_autonomy_settings_persists_max_actions_per_hour() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

    let patch = AutonomySettingsPatch {
        max_actions_per_hour: Some(500),
        ..Default::default()
    };
    let outcome = load_and_apply_autonomy_settings(patch)
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
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

    add_auto_approve_tool("git_operations")
        .await
        .expect("first add");
    // Idempotent: a second add of the same tool must not create a duplicate.
    add_auto_approve_tool("git_operations")
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
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var("OPENHUMAN_TOOL_TIMEOUT_SECS");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    let outcome = apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            agent_timeout_secs: Some(300),
            ..Default::default()
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
async fn apply_agent_settings_persists_allow_metered_agent_tools_boolean() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    // Persist true
    let outcome_true = apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            allow_metered_agent_tools: Some(true),
            ..Default::default()
        },
    )
    .await
    .expect("apply allow_metered_agent_tools=true");

    assert!(cfg.agent.allow_metered_agent_tools);
    assert_eq!(
        outcome_true.value["config"]["agent"]["allow_metered_agent_tools"],
        serde_json::json!(true)
    );
    let on_disk_true = tokio::fs::read_to_string(&cfg.config_path).await.unwrap();
    assert!(
        on_disk_true.contains("allow_metered_agent_tools = true"),
        "expected TOML to persist allow_metered_agent_tools = true, got:\n{on_disk_true}"
    );
    let on_disk_cfg_true: crate::config::Config =
        toml::from_str(&on_disk_true).expect("parse saved TOML");
    assert!(on_disk_cfg_true.agent.allow_metered_agent_tools);

    // Persist false
    let outcome_false = apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            allow_metered_agent_tools: Some(false),
            ..Default::default()
        },
    )
    .await
    .expect("apply allow_metered_agent_tools=false");

    assert!(!cfg.agent.allow_metered_agent_tools);
    assert_eq!(
        outcome_false.value["config"]["agent"]["allow_metered_agent_tools"],
        serde_json::json!(false)
    );
    let on_disk_false = tokio::fs::read_to_string(&cfg.config_path).await.unwrap();
    assert!(
        on_disk_false.contains("allow_metered_agent_tools = false"),
        "expected TOML to persist allow_metered_agent_tools = false, got:\n{on_disk_false}"
    );
    let on_disk_cfg_false: crate::config::Config =
        toml::from_str(&on_disk_false).expect("parse saved TOML");
    assert!(!on_disk_cfg_false.agent.allow_metered_agent_tools);
}

#[tokio::test]
async fn apply_agent_settings_omission_preserves_prior_allow_metered_value() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    // First, set to true
    apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            allow_metered_agent_tools: Some(true),
            ..Default::default()
        },
    )
    .await
    .expect("set allow_metered=true");
    assert!(cfg.agent.allow_metered_agent_tools);

    // Update only timeout, omitting allow_metered_agent_tools (None)
    let outcome = apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            agent_timeout_secs: Some(180),
            allow_metered_agent_tools: None,
        },
    )
    .await
    .expect("apply patch with omitted allow_metered_agent_tools");

    assert_eq!(cfg.agent.agent_timeout_secs, 180);
    assert!(
        cfg.agent.allow_metered_agent_tools,
        "omission must preserve prior true value in memory"
    );
    assert_eq!(
        outcome.value["config"]["agent"]["allow_metered_agent_tools"],
        serde_json::json!(true),
        "snapshot must preserve prior true value"
    );

    let on_disk = tokio::fs::read_to_string(&cfg.config_path).await.unwrap();
    assert!(
        on_disk.contains("allow_metered_agent_tools = true"),
        "expected TOML to preserve allow_metered_agent_tools = true on disk, got:\n{on_disk}"
    );
    let on_disk_cfg: crate::config::Config = toml::from_str(&on_disk).expect("parse saved TOML");
    assert!(
        on_disk_cfg.agent.allow_metered_agent_tools,
        "deserialized TOML must preserve prior true value"
    );
}

#[tokio::test]
async fn apply_agent_settings_getter_exposes_saved_allow_metered_boolean() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

    // Default getter value is false
    let initial = get_agent_settings().await.expect("initial get");
    assert_eq!(
        initial.value["allow_metered_agent_tools"],
        serde_json::json!(false)
    );

    // Update to true via load_and_apply_agent_settings
    load_and_apply_agent_settings(AgentSettingsPatch {
        allow_metered_agent_tools: Some(true),
        ..Default::default()
    })
    .await
    .expect("apply true");

    // Getter exposes true
    let after_true = get_agent_settings().await.expect("get after true");
    assert_eq!(
        after_true.value["allow_metered_agent_tools"],
        serde_json::json!(true)
    );

    // Update back to false
    load_and_apply_agent_settings(AgentSettingsPatch {
        allow_metered_agent_tools: Some(false),
        ..Default::default()
    })
    .await
    .expect("apply false");

    // Getter exposes false
    let after_false = get_agent_settings().await.expect("get after false");
    assert_eq!(
        after_false.value["allow_metered_agent_tools"],
        serde_json::json!(false)
    );

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}
