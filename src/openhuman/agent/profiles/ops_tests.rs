use super::*;
use crate::openhuman::agent::profiles::DEFAULT_PROFILE_ID;
use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;
use serde_json::json;

struct WorkspaceEnvGuard {
    previous: Option<std::ffi::OsString>,
}
impl WorkspaceEnvGuard {
    fn set(path: &std::path::Path) -> Self {
        let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
        }
        Self { previous }
    }
}
impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var("OPENHUMAN_WORKSPACE", value) },
            None => unsafe { std::env::remove_var("OPENHUMAN_WORKSPACE") },
        }
    }
}

struct ActionDirEnvGuard {
    previous: Option<std::ffi::OsString>,
}
impl ActionDirEnvGuard {
    fn set(path: &std::path::Path) -> Self {
        let previous = std::env::var_os("OPENHUMAN_ACTION_DIR");
        unsafe {
            std::env::set_var("OPENHUMAN_ACTION_DIR", path);
        }
        Self { previous }
    }
}
impl Drop for ActionDirEnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var("OPENHUMAN_ACTION_DIR", value) },
            None => unsafe { std::env::remove_var("OPENHUMAN_ACTION_DIR") },
        }
    }
}

fn profile(id: &str, agent_id: &str) -> AgentProfile {
    let mut p = super::super::store::built_in_default_profile();
    p.id = id.to_string();
    p.name = id.to_string();
    p.agent_id = agent_id.to_string();
    p.built_in = false;
    p.is_master = false;
    p.memory_dir_suffix = None;
    p
}

#[tokio::test]
async fn upsert_materializes_home_and_list_enriches_paths() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let ws = tempfile::tempdir().expect("ws tempdir");
    let action = tempfile::tempdir().expect("action tempdir");
    let _env_ws = WorkspaceEnvGuard::set(ws.path());
    let _env_action = ActionDirEnvGuard::set(action.path());

    let mut p = profile("writer", "orchestrator");
    p.dedicated_workspace = true;
    let out = upsert(p).await.expect("upsert");

    // The enriched payload surfaces resolved read-only paths. `soulMdFile` is
    // only inserted when the file exists on disk, so its presence proves the
    // home was materialized (workspace_dir is config-derived, so we assert on
    // the resolved suffix rather than the raw temp root). `workspaceDir` is
    // present only for a `dedicated_workspace` profile, proving the opt-in
    // dir path was resolved and created.
    let writer = out["profiles"]
        .as_array()
        .expect("profiles array")
        .iter()
        .find(|p| p["id"] == "writer")
        .expect("writer profile present");
    let soul_file = writer["soulMdFile"]
        .as_str()
        .expect("soulMdFile present (SOUL.md was seeded on disk)");
    assert!(
        soul_file.ends_with("personalities/writer/SOUL.md"),
        "soulMdFile should end at the profile home, got {soul_file}"
    );
    // The resolved SOUL.md really exists on disk.
    assert!(std::path::Path::new(soul_file).exists());
    let workspace_dir = writer["workspaceDir"]
        .as_str()
        .expect("workspaceDir present for dedicated_workspace profile");
    assert!(
        workspace_dir.ends_with("profiles/writer"),
        "workspaceDir should end at the profile workspace, got {workspace_dir}"
    );
    // The dedicated workspace dir was actually created.
    assert!(std::path::Path::new(workspace_dir).is_dir());
    assert_eq!(writer["dedicatedWorkspace"], json!(true));
    // 2a — the profile-local skills dir is seeded by `ensure_profile_home`
    // and advertised read-only as `skillsDir`.
    let skills_dir = writer["skillsDir"]
        .as_str()
        .expect("skillsDir present (skills dir was seeded on disk)");
    assert!(
        skills_dir.ends_with("personalities/writer/skills"),
        "skillsDir should end at the profile skills dir, got {skills_dir}"
    );
    assert!(std::path::Path::new(skills_dir).is_dir());
}

#[tokio::test]
async fn shared_profile_has_no_workspace_dir_in_payload() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let ws = tempfile::tempdir().expect("ws tempdir");
    let action = tempfile::tempdir().expect("action tempdir");
    let _env_ws = WorkspaceEnvGuard::set(ws.path());
    let _env_action = ActionDirEnvGuard::set(action.path());

    // A shared-workspace profile: soulMdFile resolves (home is seeded) but no
    // workspaceDir key is present.
    let out = upsert(profile("shared", "orchestrator"))
        .await
        .expect("upsert");
    let shared = out["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "shared")
        .expect("shared profile present");
    // A shared-workspace profile never surfaces workspaceDir, but its home
    // (SOUL.md) is still seeded, so soulMdFile resolves.
    assert!(shared.get("workspaceDir").is_none());
    assert!(shared["soulMdFile"].as_str().is_some());
}

#[test]
fn enrich_profile_skips_paths_for_invalid_id() {
    let ws = tempfile::tempdir().expect("ws tempdir");
    let action = tempfile::tempdir().expect("action tempdir");
    let mut p = profile("placeholder", "orchestrator");
    p.id = "Bad Id".to_string();
    p.dedicated_workspace = true;

    // Even if a SOUL.md somehow exists at the would-be home, an invalid id must
    // not advertise soulMdFile/workspaceDir — the read paths would never load
    // it, so the enriched payload must stay symmetric with what core reads.
    let home = super::profile_home(ws.path(), "Bad Id");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("SOUL.md"), "x").unwrap();

    let enriched = super::enrich_profile(ws.path(), action.path(), &p);
    assert!(
        enriched.get("soulMdFile").is_none(),
        "invalid id must not advertise soulMdFile even when the file exists"
    );
    assert!(
        enriched.get("workspaceDir").is_none(),
        "invalid id must not advertise workspaceDir"
    );

    // Control: a valid id with the same on-disk SOUL.md does advertise it.
    let mut valid = p.clone();
    valid.id = "goodid".to_string();
    let valid_home = super::profile_home(ws.path(), "goodid");
    std::fs::create_dir_all(&valid_home).unwrap();
    std::fs::write(valid_home.join("SOUL.md"), "x").unwrap();
    let enriched_valid = super::enrich_profile(ws.path(), action.path(), &valid);
    assert!(enriched_valid["soulMdFile"].as_str().is_some());
    assert!(enriched_valid["workspaceDir"].as_str().is_some());
}

#[tokio::test]
async fn upsert_default_agent_allowed_without_registry() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let _env = WorkspaceEnvGuard::set(temp.path());
    // orchestrator is always valid even with no registry initialised.
    let out = upsert(profile("writer", "orchestrator"))
        .await
        .expect("upsert");
    assert!(out["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["id"] == "writer"));
}

#[tokio::test]
async fn upsert_unknown_agent_is_rejected() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let _env = WorkspaceEnvGuard::set(temp.path());
    // A bogus non-default agent_id must never persist. Depending on whether
    // another test already initialised the process-global registry, the
    // rejection comes either from registry validation ("not found") or from
    // the fail-closed no-registry path ("registry unavailable") — both are
    // acceptable; the invariant is that it errors rather than saving.
    let err = upsert(profile("bad", "__missing_agent__"))
        .await
        .expect_err("must reject unknown agent");
    assert!(
        err.contains("registry unavailable") || err.contains("not found"),
        "err: {err}"
    );
}

#[tokio::test]
async fn list_select_delete_roundtrip() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let _env = WorkspaceEnvGuard::set(temp.path());
    upsert(profile("writer", "orchestrator"))
        .await
        .expect("upsert");
    let selected = select("writer").await.expect("select");
    assert_eq!(selected["activeProfileId"], "writer");
    let listed = list().await.expect("list");
    assert_eq!(listed["activeProfileId"], "writer");
    let deleted = delete("writer").await.expect("delete");
    assert_eq!(deleted["activeProfileId"], json!(DEFAULT_PROFILE_ID));
}
