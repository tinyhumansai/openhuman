//! The files folder setting (#5505): where agent deliverables are written.

use super::*;

fn patch(files_dir: &str) -> AgentPathsPatch {
    AgentPathsPatch {
        action_dir: None,
        files_dir: Some(files_dir.to_string()),
    }
}

#[tokio::test]
async fn setting_the_files_folder_persists_it_and_creates_it() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let chosen = tmp.path().join("Deliverables");

    let outcome = apply_agent_paths_settings(&mut cfg, patch(&chosen.to_string_lossy()))
        .await
        .expect("apply files_dir");

    assert!(chosen.is_dir(), "a missing folder is created");
    assert_eq!(cfg.files_dir_override.as_deref(), Some(chosen.as_path()));
    assert_eq!(cfg.files_dir(), chosen);
    assert_eq!(
        outcome.value["files_dir"],
        serde_json::json!(chosen.display().to_string())
    );
    assert_eq!(
        outcome.value["files_dir_source"],
        serde_json::json!("override")
    );
    let saved = std::fs::read_to_string(&cfg.config_path).expect("config saved");
    assert!(saved.contains("files_dir_override"), "{saved}");
}

#[tokio::test]
async fn an_empty_value_reverts_to_the_default_folder() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.files_dir_override = Some(tmp.path().join("Old"));

    let outcome = apply_agent_paths_settings(&mut cfg, patch("   "))
        .await
        .expect("clear files_dir");

    assert!(cfg.files_dir_override.is_none());
    assert_eq!(cfg.files_dir(), crate::config::default_files_dir());
    assert_eq!(
        outcome.value["files_dir_source"],
        serde_json::json!("default")
    );
    assert_eq!(
        outcome.value["default_files_dir"],
        serde_json::json!(crate::config::default_files_dir().display().to_string())
    );
}

async fn rejected(cfg: &mut Config, value: &str) -> String {
    let before = cfg.files_dir_override.clone();
    let err = apply_agent_paths_settings(cfg, patch(value))
        .await
        .expect_err("files_dir must be rejected");
    assert_eq!(
        cfg.files_dir_override, before,
        "a rejected value is not stored"
    );
    err
}

#[tokio::test]
async fn a_relative_path_is_rejected() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let err = rejected(&mut cfg, "relative/Files").await;
    assert!(err.contains("absolute"), "{err}");
}

#[tokio::test]
async fn a_file_is_rejected() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let file = tmp.path().join("not-a-folder.txt");
    std::fs::write(&file, b"x").unwrap();
    let err = rejected(&mut cfg, &file.to_string_lossy()).await;
    assert!(err.contains("not a file"), "{err}");
}

#[tokio::test]
async fn a_folder_inside_the_openhuman_data_folder_is_rejected() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let inside = cfg.workspace_dir.join("Files");
    let err = rejected(&mut cfg, &inside.to_string_lossy()).await;
    assert!(err.contains("OpenHuman data folder"), "{err}");
    assert!(!inside.exists(), "nothing is created for a rejected value");
}

#[tokio::test]
async fn a_credential_folder_is_rejected() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let ssh = tmp.path().join(".ssh").join("Files");
    let err = rejected(&mut cfg, &ssh.to_string_lossy()).await;
    assert!(err.contains("protected"), "{err}");
}

#[test]
fn a_relative_override_on_disk_falls_back_to_the_default() {
    let cfg = Config {
        files_dir_override: Some(PathBuf::from("relative")),
        ..Config::default()
    };
    assert_eq!(cfg.files_dir(), crate::config::default_files_dir());
}

/// The boot migration moves legacy files into the folder the user chose, not
/// the default one.
#[tokio::test]
async fn boot_migrates_legacy_files_into_the_chosen_folder() {
    use crate::agent::artifacts::store::save_artifact_meta;
    use crate::agent::artifacts::{ArtifactKind, ArtifactMeta, ArtifactStatus};

    let _g = ENV_LOCK.lock().await;
    let tmp = tempdir().unwrap();
    let prev_projects_dir = std::env::var_os("OPENHUMAN_PROJECTS_DIR");
    unsafe {
        std::env::set_var("OPENHUMAN_PROJECTS_DIR", tmp.path().join("projects-home"));
    }

    let mut cfg = tmp_config(&tmp);
    cfg.action_dir = tmp.path().join("action");
    let chosen = tmp.path().join("Chosen");
    cfg.files_dir_override = Some(chosen.clone());
    let meta = ArtifactMeta {
        id: "legacy".to_string(),
        kind: ArtifactKind::Document,
        title: "Plan".to_string(),
        path: "legacy/plan.docx".to_string(),
        file: None,
        file_root: None,
        size_bytes: 4,
        status: ArtifactStatus::Ready,
        created_at: chrono::Utc::now(),
        error: None,
        thread_id: None,
        tool_call_id: None,
    };
    save_artifact_meta(&cfg.workspace_dir, &meta).await.unwrap();
    // The record lives in the document store now, so the legacy content
    // directory is created by hand, as an older install left it.
    std::fs::create_dir_all(cfg.workspace_dir.join("artifacts/legacy")).unwrap();
    std::fs::write(
        cfg.workspace_dir.join("artifacts/legacy/plan.docx"),
        b"plan",
    )
    .unwrap();

    crate::config::ensure_agent_dirs(&mut cfg).await;

    unsafe {
        match prev_projects_dir {
            Some(v) => std::env::set_var("OPENHUMAN_PROJECTS_DIR", v),
            None => std::env::remove_var("OPENHUMAN_PROJECTS_DIR"),
        }
    }
    assert_eq!(std::fs::read(chosen.join("plan.docx")).unwrap(), b"plan");
}

/// A symlinked spelling of the data folder is still the data folder: the
/// check compares canonical paths, not text.
#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_route_into_the_data_folder_is_rejected() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let alias = tmp.path().join("alias");
    std::os::unix::fs::symlink(&cfg.workspace_dir, &alias).unwrap();

    let err = rejected(&mut cfg, &alias.join("Files").to_string_lossy()).await;
    assert!(err.contains("OpenHuman data folder"), "{err}");
}

/// Changing the folder affects new files only: a file made in the previous
/// folder keeps resolving, because that folder is recorded as trusted.
#[tokio::test]
async fn files_made_before_a_folder_change_keep_resolving() {
    use crate::agent::artifacts::store::{create_artifact, finalize_artifact};
    use crate::agent::artifacts::{ArtifactKind, FileRoots};

    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let first = tmp.path().join("First");
    let second = tmp.path().join("Second");
    apply_agent_paths_settings(&mut cfg, patch(&first.to_string_lossy()))
        .await
        .unwrap();
    let (meta, path) = create_artifact(
        &cfg.workspace_dir,
        FileRoots::from_config(&cfg),
        ArtifactKind::Document,
        "Plan",
        "docx",
        None,
    )
    .await
    .unwrap();
    std::fs::write(&path, b"plan").unwrap();
    finalize_artifact(&cfg.workspace_dir, &meta.id, 4, None)
        .await
        .unwrap();
    assert!(path.starts_with(&first));

    apply_agent_paths_settings(&mut cfg, patch(&second.to_string_lossy()))
        .await
        .unwrap();

    assert!(
        cfg.files_dir_history.contains(&first),
        "{:?}",
        cfg.files_dir_history
    );
    let value = crate::agent::artifacts::ops::ai_get_artifact(&cfg, &meta.id)
        .await
        .expect("an artifact from the previous folder still resolves")
        .into_cli_compatible_json()
        .unwrap();
    assert_eq!(
        value["absolute_path"],
        serde_json::json!(path.to_string_lossy())
    );
}
