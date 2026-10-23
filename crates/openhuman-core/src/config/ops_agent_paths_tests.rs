use super::*;

#[tokio::test]
async fn apply_agent_settings_rejects_out_of_range_timeout() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let original = cfg.agent.agent_timeout_secs;

    // Zero would disable the timeout — rejected.
    let err = apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            agent_timeout_secs: Some(0),
            ..AgentSettingsPatch::default()
        },
    )
    .await
    .expect_err("zero timeout should be rejected");
    assert!(err.contains("between"), "unexpected error: {err}");

    // Above the 3600s ceiling — rejected.
    let err = apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            agent_timeout_secs: Some(99_999),
            ..AgentSettingsPatch::default()
        },
    )
    .await
    .expect_err("over-max timeout should be rejected");
    assert!(err.contains("between"), "unexpected error: {err}");

    // The config value is untouched after a rejected update.
    assert_eq!(cfg.agent.agent_timeout_secs, original);
}

#[tokio::test]
async fn apply_agent_settings_none_leaves_timeout_unchanged() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.agent.agent_timeout_secs = 250;

    apply_agent_settings(&mut cfg, AgentSettingsPatch::default())
        .await
        .expect("apply no-op agent settings");

    assert_eq!(cfg.agent.agent_timeout_secs, 250);
}

#[tokio::test]
async fn apply_agent_settings_rejects_unknown_chat_agent_id() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    let err = apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            chat_agent_id: Some("typoed_agent".into()),
            ..AgentSettingsPatch::default()
        },
    )
    .await
    .expect_err("unknown agents must not be persisted as web-chat routes");

    assert!(err.contains("not a runnable agent definition"), "{err}");
    assert!(cfg.agent.chat_agent_id.is_none());
}

#[tokio::test]
async fn apply_agent_settings_blank_chat_agent_id_clears_and_persists_override() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.agent.chat_agent_id = Some("researcher".into());

    let outcome = apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            chat_agent_id: Some("   ".into()),
            ..AgentSettingsPatch::default()
        },
    )
    .await
    .expect("blank chat agent id clears the override");

    assert_eq!(cfg.agent.chat_agent_id, None);
    assert_eq!(
        outcome.value["config"]["agent"]["chat_agent_id"],
        serde_json::Value::Null
    );

    let saved = tokio::fs::read_to_string(&cfg.config_path)
        .await
        .expect("saved config");
    assert!(
        !saved.contains("chat_agent_id"),
        "cleared override must not remain in the persisted config: {saved}"
    );
}

#[tokio::test]
async fn apply_agent_settings_rejects_a_mixed_patch_without_mutating_config() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let original_timeout = cfg.agent.agent_timeout_secs;

    let err = apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            agent_timeout_secs: Some(300),
            chat_agent_id: Some("typoed_agent".into()),
            ..AgentSettingsPatch::default()
        },
    )
    .await
    .expect_err("unknown agent must reject the entire patch");

    assert!(err.contains("not a runnable agent definition"), "{err}");
    assert_eq!(cfg.agent.agent_timeout_secs, original_timeout);
    assert!(cfg.agent.chat_agent_id.is_none());
}

// ── apply_agent_paths_settings (action_dir editable, issue #3240) ──────────────

#[tokio::test]
async fn apply_agent_paths_valid_abs_path_persists_override_and_recomputes() {
    let _g = ENV_LOCK.lock().await;
    // Ensure no env override is interfering.
    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let new_dir = tmp.path().join("agent-projects");
    std::fs::create_dir_all(&new_dir).unwrap();

    let outcome = apply_agent_paths_settings(
        &mut cfg,
        AgentPathsPatch {
            action_dir: Some(new_dir.to_string_lossy().to_string()),
            files_dir: None,
        },
    )
    .await
    .expect("apply agent paths");

    assert_eq!(cfg.action_dir_override.as_deref(), Some(new_dir.as_path()));
    assert_eq!(cfg.action_dir, new_dir);
    assert_eq!(
        outcome.value["action_dir"],
        serde_json::json!(new_dir.display().to_string())
    );
    assert_eq!(
        outcome.value["action_dir_source"],
        serde_json::json!("override")
    );
}

#[tokio::test]
async fn apply_agent_paths_rejects_relative_path() {
    let _g = ENV_LOCK.lock().await;
    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    let err = apply_agent_paths_settings(
        &mut cfg,
        AgentPathsPatch {
            action_dir: Some("relative/projects".into()),
            files_dir: None,
        },
    )
    .await
    .expect_err("relative path must be rejected");

    assert!(err.contains("absolute"), "unexpected error: {err}");
    assert!(cfg.action_dir_override.is_none());
}

#[tokio::test]
async fn apply_agent_paths_rejects_action_dir_equal_to_workspace() {
    let _g = ENV_LOCK.lock().await;
    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let workspace = cfg.workspace_dir.clone();

    let err = apply_agent_paths_settings(
        &mut cfg,
        AgentPathsPatch {
            action_dir: Some(workspace.to_string_lossy().to_string()),
            files_dir: None,
        },
    )
    .await
    .expect_err("action_dir == workspace_dir must be rejected");

    assert!(err.contains("workspace"), "unexpected error: {err}");
    assert!(cfg.action_dir_override.is_none());
}

#[tokio::test]
async fn apply_agent_paths_empty_input_clears_override() {
    let _g = ENV_LOCK.lock().await;
    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    // Start with an override in place.
    let prior = tmp.path().join("prior-projects");
    std::fs::create_dir_all(&prior).unwrap();
    cfg.action_dir_override = Some(prior);

    let outcome = apply_agent_paths_settings(
        &mut cfg,
        AgentPathsPatch {
            action_dir: Some("   ".into()),
            files_dir: None,
        },
    )
    .await
    .expect("clear override");

    assert!(cfg.action_dir_override.is_none());
    // Reverts to the default projects dir.
    assert_eq!(cfg.action_dir, crate::config::default_projects_dir());
    assert_eq!(
        outcome.value["action_dir_source"],
        serde_json::json!("default")
    );
}

#[tokio::test]
async fn apply_agent_paths_auto_creates_missing_directory() {
    let _g = ENV_LOCK.lock().await;
    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let missing = tmp.path().join("not-yet").join("created");
    assert!(!missing.exists());

    apply_agent_paths_settings(
        &mut cfg,
        AgentPathsPatch {
            action_dir: Some(missing.to_string_lossy().to_string()),
            files_dir: None,
        },
    )
    .await
    .expect("auto-create action dir");

    assert!(missing.is_dir(), "missing action_dir must be auto-created");
    assert_eq!(cfg.action_dir, missing);
}

#[tokio::test]
async fn apply_agent_paths_rejects_existing_file() {
    let _g = ENV_LOCK.lock().await;
    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let file = tmp.path().join("a-file.txt");
    std::fs::write(&file, b"not a dir").unwrap();

    let err = apply_agent_paths_settings(
        &mut cfg,
        AgentPathsPatch {
            action_dir: Some(file.to_string_lossy().to_string()),
            files_dir: None,
        },
    )
    .await
    .expect_err("a file path must be rejected");

    assert!(err.contains("directory"), "unexpected error: {err}");
    assert!(cfg.action_dir_override.is_none());
}

#[tokio::test]
async fn apply_agent_paths_env_set_reports_source_env() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let env_dir = tmp.path().join("env-pinned");
    std::fs::create_dir_all(&env_dir).unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_ACTION_DIR", &env_dir);
    }

    let mut cfg = tmp_config(&tmp);
    // Even when the user sets an override, the env var wins for the effective
    // value and the reported source.
    let user_dir = tmp.path().join("user-choice");
    std::fs::create_dir_all(&user_dir).unwrap();

    let outcome = apply_agent_paths_settings(
        &mut cfg,
        AgentPathsPatch {
            action_dir: Some(user_dir.to_string_lossy().to_string()),
            files_dir: None,
        },
    )
    .await
    .expect("apply with env override present");

    // Override is persisted, but the effective action_dir reflects the env.
    assert_eq!(cfg.action_dir_override.as_deref(), Some(user_dir.as_path()));
    assert_eq!(cfg.action_dir, env_dir);
    assert_eq!(outcome.value["action_dir_source"], serde_json::json!("env"));

    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
    }
}

// --- #3353 regression tests -------------------------------------------------

#[test]
fn expand_tilde_happy_path_uses_home() {
    // `~/OpenHuman/projects` resolves to the home dir joined component-wise.
    let expanded = expand_tilde("~/OpenHuman/projects");
    let expected = dirs::home_dir()
        .expect("home dir resolvable in test env")
        .join("OpenHuman")
        .join("projects");
    assert_eq!(expanded, expected.to_string_lossy());
}

#[test]
fn expand_tilde_without_prefix_is_unchanged() {
    // Absolute paths and a bare `~` (no trailing slash) pass through verbatim.
    assert_eq!(expand_tilde("/abs/path"), "/abs/path");
    assert_eq!(expand_tilde("~"), "~");
    assert_eq!(expand_tilde("relative/path"), "relative/path");
}

#[cfg(windows)]
#[test]
fn expand_tilde_has_no_mixed_separators_on_windows() {
    // The whole point of the component-wise build: the result must be a pure
    // backslash path with no embedded forward slash, so CreateProcessW accepts
    // it as a CWD instead of failing with ERROR_DIRECTORY (os error 267).
    let expanded = expand_tilde("~/OpenHuman/projects");
    assert!(
        !expanded.contains('/'),
        "expected no forward slashes on Windows, got: {expanded}"
    );
}

#[test]
fn redact_home_suppresses_paths_when_tinysecurity_is_unavailable() {
    let home = dirs::home_dir().expect("home dir resolvable in test env");

    let under_home = home.join("OpenHuman").join("projects");
    let outside = std::path::Path::new("/var/lib/openhuman/action");
    assert_eq!(
        redact_home(&under_home),
        crate::core::log_redaction::SUPPRESSED
    );
    assert_eq!(redact_home(outside), crate::core::log_redaction::SUPPRESSED);
}

#[test]
fn redacted_windows_paths_use_forward_slashes_for_display() {
    assert_eq!(
        crate::config::ops::agent_paths::normalize_redacted_path(
            r"~\OpenHuman\projects".to_owned()
        ),
        "~/OpenHuman/projects"
    );
}

#[tokio::test]
async fn ensure_agent_dirs_creates_missing_action_dir_and_trusted_root() {
    use crate::security::TrustedAccess;

    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    // Point the default projects home at the tempdir so the helper doesn't touch
    // the real `~/OpenHuman/projects`.
    let projects_dir = tmp.path().join("projects-home");
    let prev_projects_dir = std::env::var_os("OPENHUMAN_PROJECTS_DIR");
    unsafe {
        std::env::set_var("OPENHUMAN_PROJECTS_DIR", &projects_dir);
    }

    let mut cfg = tmp_config(&tmp);
    let action_dir = tmp.path().join("fresh-action-dir");
    cfg.action_dir = action_dir.clone();
    assert!(!action_dir.exists(), "precondition: action_dir is missing");

    crate::config::ensure_agent_dirs(&mut cfg).await;

    // Both the action_dir and the projects home now exist.
    assert!(action_dir.is_dir(), "action_dir should be created");
    assert!(projects_dir.is_dir(), "projects home should be created");

    // The projects home is registered exactly once as a ReadWrite trusted root.
    let projects_path = projects_dir.to_string_lossy().to_string();
    let matching: Vec<_> = cfg
        .security
        .autonomy
        .trusted_roots
        .iter()
        .filter(|r| r.path == projects_path)
        .collect();
    assert_eq!(matching.len(), 1, "trusted root registered exactly once");
    assert!(matches!(matching[0].access, TrustedAccess::ReadWrite));

    // Idempotent: a second call neither errors nor duplicates the trusted root.
    crate::config::ensure_agent_dirs(&mut cfg).await;
    let count = cfg
        .security
        .autonomy
        .trusted_roots
        .iter()
        .filter(|r| r.path == projects_path)
        .count();
    assert_eq!(count, 1, "second call must not duplicate the trusted root");

    // Restore the prior env state so later tests observe the real environment.
    unsafe {
        match prev_projects_dir {
            Some(v) => std::env::set_var("OPENHUMAN_PROJECTS_DIR", v),
            None => std::env::remove_var("OPENHUMAN_PROJECTS_DIR"),
        }
    }
}

#[test]
fn ensure_usable_cwd_creates_missing_dir() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path().join("not-yet-here");
    assert!(!dir.exists());
    crate::config::ensure_usable_cwd(&dir).expect("missing dir is created");
    assert!(dir.is_dir());
}

#[test]
fn ensure_usable_cwd_rejects_a_file_with_descriptive_error() {
    let tmp = tempdir().unwrap();
    let file = tmp.path().join("a-file");
    std::fs::write(&file, b"x").unwrap();
    let err = crate::config::ensure_usable_cwd(&file)
        .expect_err("an existing file is not a usable working directory");
    let msg = err.to_string();
    assert!(msg.contains("not a directory"), "unexpected error: {msg}");
    // The error names the offending path so the message is actionable.
    assert!(
        msg.contains(&file.to_string_lossy().to_string()),
        "error should name the path: {msg}"
    );
}

#[test]
fn ensure_usable_cwd_errors_when_uncreatable() {
    // A directory whose parent is an existing *file* can't be created; the
    // helper must surface the descriptive, path-naming error rather than panic.
    let tmp = tempdir().unwrap();
    let parent_file = tmp.path().join("parent-file");
    std::fs::write(&parent_file, b"x").unwrap();
    let target = parent_file.join("child");
    let err =
        crate::config::ensure_usable_cwd(&target).expect_err("cannot create a dir under a file");
    let msg = err.to_string();
    assert!(
        msg.contains("could not be created"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn apply_agent_settings_sets_and_normalizes_tool_dispatcher() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    assert_eq!(
        cfg.agent.tool_dispatcher, "auto",
        "JSON/native is the default"
    );

    apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            tool_dispatcher: Some("  Python ".into()),
            ..AgentSettingsPatch::default()
        },
    )
    .await
    .expect("known dispatcher is accepted");
    assert_eq!(cfg.agent.tool_dispatcher, "python");

    // Omitting the field leaves it alone.
    apply_agent_settings(&mut cfg, AgentSettingsPatch::default())
        .await
        .expect("no-op");
    assert_eq!(cfg.agent.tool_dispatcher, "python");
}

#[tokio::test]
async fn apply_agent_settings_rejects_unknown_tool_dispatcher_without_mutating() {
    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    let err = apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            agent_timeout_secs: Some(321),
            tool_dispatcher: Some("cobol".into()),
            ..AgentSettingsPatch::default()
        },
    )
    .await
    .expect_err("unknown dispatcher must be rejected");
    assert!(err.contains("invalid tool_dispatcher"), "unexpected: {err}");
    assert_eq!(cfg.agent.tool_dispatcher, "auto");
    assert_ne!(cfg.agent.agent_timeout_secs, 321, "mixed patch is atomic");
}
