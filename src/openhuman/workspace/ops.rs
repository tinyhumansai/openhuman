use serde_json::json;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::heartbeat::engine::HeartbeatEngine;
use crate::openhuman::skills::init_skills_dir;
use std::path::Path;

const BOOTSTRAP_FILES: [(&str, &str); 2] = [
    ("SOUL.md", include_str!("../agent/prompts/SOUL.md")),
    ("IDENTITY.md", include_str!("../agent/prompts/IDENTITY.md")),
];

fn ensure_workspace_file(
    workspace_dir: &Path,
    filename: &str,
    contents: &str,
    force: bool,
) -> Result<&'static str, String> {
    let path = workspace_dir.join(filename);
    if path.exists() && !force {
        return Ok("existing");
    }
    std::fs::write(&path, contents)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(if force { "overwritten" } else { "created" })
}

/// Create default dirs, copy bundled prompts, skills README, and heartbeat file.
pub async fn init_workspace(force: bool) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let workspace_dir = config.workspace_dir.clone();

    let mut created_dirs = Vec::new();
    let mut existing_dirs = Vec::new();
    for rel in ["memory", "sessions", "state", "cron"] {
        let dir = workspace_dir.join(rel);
        if dir.exists() {
            existing_dirs.push(dir.display().to_string());
        } else {
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("failed to create directory {}: {e}", dir.display()))?;
            created_dirs.push(dir.display().to_string());
        }
    }

    let mut created_files = Vec::new();
    let mut overwritten_files = Vec::new();
    let mut existing_files = Vec::new();
    for (filename, contents) in BOOTSTRAP_FILES {
        match ensure_workspace_file(&workspace_dir, filename, contents, force)? {
            "created" => created_files.push(workspace_dir.join(filename).display().to_string()),
            "overwritten" => {
                overwritten_files.push(workspace_dir.join(filename).display().to_string())
            }
            _ => existing_files.push(workspace_dir.join(filename).display().to_string()),
        }
    }

    let skills_readme = workspace_dir.join("skills").join("README.md");
    let had_skills_readme = skills_readme.exists();
    let heartbeat = workspace_dir.join("HEARTBEAT.md");
    let had_heartbeat = heartbeat.exists();
    init_skills_dir(&workspace_dir).map_err(|e| format!("failed to initialize skills dir: {e}"))?;
    HeartbeatEngine::ensure_heartbeat_file(&workspace_dir)
        .await
        .map_err(|e| format!("failed to initialize HEARTBEAT.md: {e}"))?;

    if had_skills_readme {
        existing_files.push(skills_readme.display().to_string());
    } else {
        created_files.push(skills_readme.display().to_string());
    }

    if had_heartbeat {
        existing_files.push(heartbeat.display().to_string());
    } else {
        created_files.push(heartbeat.display().to_string());
    }

    Ok(json!({
        "result": {
            "workspace_dir": workspace_dir.display().to_string(),
            "config_path": config.config_path.display().to_string(),
            "directories": {
                "created": created_dirs,
                "existing": existing_dirs
            },
            "files": {
                "created": created_files,
                "overwritten": overwritten_files,
                "existing": existing_files
            }
        },
        "logs": [
            "workspace initialization completed"
        ]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;
    use tempfile::tempdir;

    // ── ensure_workspace_file ──────────────────────────────────────

    #[test]
    fn ensure_workspace_file_creates_missing_file() {
        let tmp = tempdir().unwrap();
        let status =
            ensure_workspace_file(tmp.path(), "A.md", "hello", false).expect("should create");
        assert_eq!(status, "created");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("A.md")).unwrap(),
            "hello"
        );
    }

    #[test]
    fn ensure_workspace_file_leaves_existing_file_untouched_without_force() {
        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("B.md"), "original").unwrap();
        let status = ensure_workspace_file(tmp.path(), "B.md", "new contents", false).expect("ok");
        assert_eq!(status, "existing");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("B.md")).unwrap(),
            "original",
            "file must not be overwritten when force=false"
        );
    }

    #[test]
    fn ensure_workspace_file_overwrites_when_forced() {
        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("C.md"), "original").unwrap();
        let status = ensure_workspace_file(tmp.path(), "C.md", "new contents", true).expect("ok");
        assert_eq!(status, "overwritten");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("C.md")).unwrap(),
            "new contents"
        );
    }

    #[test]
    fn ensure_workspace_file_errors_when_directory_missing() {
        let tmp = tempdir().unwrap();
        let missing = tmp.path().join("does/not/exist");
        let err = ensure_workspace_file(&missing, "x.md", "y", false).unwrap_err();
        assert!(
            err.contains("failed to write"),
            "expected write-failure error, got: {err}"
        );
    }

    #[test]
    fn bootstrap_files_contain_soul_and_identity() {
        // Lock in the contract so `init_workspace` doesn't silently stop
        // shipping a required prompt. These are the canonical prompt
        // files the agent harness expects in every fresh workspace.
        let names: Vec<&str> = BOOTSTRAP_FILES.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"SOUL.md"));
        assert!(names.contains(&"IDENTITY.md"));
        assert_eq!(BOOTSTRAP_FILES.len(), 2);
        // Bundled contents must be non-empty — a packaging regression
        // that empties one would otherwise silently ship a broken agent.
        for (_, contents) in BOOTSTRAP_FILES {
            assert!(!contents.trim().is_empty());
        }
    }

    // ── init_workspace ────────────────────────────────────────────

    #[tokio::test]
    async fn init_workspace_creates_dirs_and_files_in_fresh_workspace() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }

        let result = init_workspace(false).await;

        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
        let value = result.expect("init_workspace on empty temp should succeed");

        let workspace_dir = value["result"]["workspace_dir"]
            .as_str()
            .expect("workspace_dir string");
        let workspace_dir = std::path::PathBuf::from(workspace_dir);
        for rel in ["memory", "sessions", "state", "cron"] {
            assert!(
                workspace_dir.join(rel).is_dir(),
                "expected {rel} directory under {}",
                workspace_dir.display()
            );
        }
        assert!(workspace_dir.join("SOUL.md").is_file());
        assert!(workspace_dir.join("IDENTITY.md").is_file());
        assert!(workspace_dir.join("HEARTBEAT.md").is_file());

        let created: Vec<&str> = value["result"]["files"]["created"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(created.iter().any(|s| s.ends_with("SOUL.md")));
        assert!(created.iter().any(|s| s.ends_with("IDENTITY.md")));

        let logs = value["logs"].as_array().expect("logs array");
        assert!(logs.iter().any(|l| l
            .as_str()
            .unwrap_or("")
            .contains("workspace initialization completed")));
    }

    #[tokio::test]
    async fn init_workspace_reports_existing_entries_on_second_call_without_force() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }

        // First call populates the workspace.
        init_workspace(false).await.expect("first init ok");
        // Second call without force should report everything as existing
        // and nothing as created / overwritten.
        let value = init_workspace(false).await.expect("second init ok");

        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }

        let created = value["result"]["files"]["created"].as_array().unwrap();
        let overwritten = value["result"]["files"]["overwritten"].as_array().unwrap();
        let existing = value["result"]["files"]["existing"].as_array().unwrap();
        assert!(created.is_empty(), "no files should be re-created");
        assert!(overwritten.is_empty(), "no files should be overwritten");
        assert!(
            existing
                .iter()
                .any(|v| v.as_str().unwrap_or("").ends_with("SOUL.md")),
            "SOUL.md should appear in the existing list"
        );

        let created_dirs = value["result"]["directories"]["created"]
            .as_array()
            .unwrap();
        let existing_dirs = value["result"]["directories"]["existing"]
            .as_array()
            .unwrap();
        assert!(created_dirs.is_empty());
        assert!(!existing_dirs.is_empty());
    }

    #[tokio::test]
    async fn init_workspace_with_force_overwrites_existing_bootstrap_files() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }

        let first = init_workspace(false).await.expect("initial init");
        // The config loader may place the workspace at a subpath of the
        // env override (e.g. `{tmp}/workspace`), so discover the real
        // location from the first result rather than assuming it is
        // `tmp.path()` itself.
        let workspace_dir = std::path::PathBuf::from(
            first["result"]["workspace_dir"]
                .as_str()
                .expect("workspace_dir string"),
        );
        let soul = workspace_dir.join("SOUL.md");
        std::fs::write(&soul, "corrupted").unwrap();

        let value = init_workspace(true).await.expect("forced init");

        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }

        let overwritten: Vec<&str> = value["result"]["files"]["overwritten"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(overwritten.iter().any(|s| s.ends_with("SOUL.md")));
        // And the on-disk contents must no longer be "corrupted".
        let restored = std::fs::read_to_string(&soul).unwrap();
        assert_ne!(restored, "corrupted");
        assert!(!restored.trim().is_empty());
    }
}
