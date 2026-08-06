//! Business logic for the `profiles` RPC surface.
//!
//! Per the OpenHuman domain contract, controller handlers in [`super::schemas`]
//! stay thin (deserialize + delegate) and the real work — config loading,
//! `agent_id` validation, and store mutation — lives here, returning the JSON
//! payload the controller emits. The persistence itself is owned by
//! [`AgentProfileStore`](super::store::AgentProfileStore).

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::home::{
    dedicated_workspace_dir, ensure_profile_home, profile_home, validate_profile_id,
};
use super::store::AgentProfileStore;
use super::types::{AgentProfile, AgentProfilesState};
use crate::openhuman::config::rpc as config_rpc;

/// Enrich one stored profile with resolved, read-only path info the UI shows but
/// the [`AgentProfile`] struct never persists (section E):
/// - `soulMdFile`: absolute path of `personalities/<id>/SOUL.md` when it exists;
/// - `workspaceDir`: absolute path of the dedicated workspace when opted in.
///
/// Derived on read from the two path roots — never round-tripped back into the
/// store, so the stored payload stays clean.
fn enrich_profile(workspace_dir: &Path, action_dir: &Path, profile: &AgentProfile) -> Value {
    let mut obj: Map<String, Value> = match serde_json::to_value(profile) {
        Ok(Value::Object(map)) => map,
        _ => {
            // A profile must serialize to a JSON object; anything else drops every
            // field. This should be unreachable for `AgentProfile`, so warn (but
            // keep the empty-map fallback so the RPC still returns a value).
            tracing::warn!(
                profile_id = %profile.id,
                "[profiles][ops] enrich_profile: profile did not serialize to a JSON \
                 object; enrichment yields an empty object"
            );
            Map::new()
        }
    };
    // Skip the derived path enrichment for ids the read paths would never load
    // (they fail `validate_profile_id`). `dedicated_workspace_dir` already gates
    // on the same check, so `workspaceDir` was never emitted for them; matching
    // that for `soulMdFile` keeps the advertised paths symmetric with what the
    // core will actually read, and — paired with the `ensure_profile_home` guard
    // — such an id has no SOUL.md on disk to advertise anyway.
    if validate_profile_id(&profile.id).is_err() {
        return Value::Object(obj);
    }
    let soul = profile_home(workspace_dir, &profile.id).join("SOUL.md");
    if soul.exists() {
        obj.insert(
            "soulMdFile".to_string(),
            Value::String(soul.to_string_lossy().into_owned()),
        );
    }
    // 2a — advertise the profile-local skills dir when it exists on disk (seeded
    // by `ensure_profile_home`). Read-only, derived, never persisted. The UI
    // (Phase 3) surfaces it as "skills placed here are private to this profile".
    let skills_dir = super::home::profile_skills_dir(workspace_dir, &profile.id);
    if skills_dir.is_dir() {
        obj.insert(
            "skillsDir".to_string(),
            Value::String(skills_dir.to_string_lossy().into_owned()),
        );
    }
    if let Some(ws) = dedicated_workspace_dir(action_dir, profile) {
        obj.insert(
            "workspaceDir".to_string(),
            Value::String(ws.to_string_lossy().into_owned()),
        );
    }
    Value::Object(obj)
}

/// Shape the `{ profiles, activeProfileId }` payload every profiles RPC returns,
/// enriching each profile with its resolved read-only path info.
fn enriched_state_payload(
    workspace_dir: &Path,
    action_dir: &Path,
    state: &AgentProfilesState,
) -> Value {
    let profiles: Vec<Value> = state
        .profiles
        .iter()
        .map(|p| enrich_profile(workspace_dir, action_dir, p))
        .collect();
    serde_json::json!({
        "profiles": profiles,
        "activeProfileId": state.active_profile_id,
    })
}

/// Resolve the workspace-scoped profile store plus the `(workspace, action)`
/// path roots from a single config load, so the id-scoped store call, the home
/// materialization, and payload enrichment all share one load.
async fn store_and_roots() -> Result<(AgentProfileStore, PathBuf, PathBuf), String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let store = AgentProfileStore::new(config.workspace_dir.clone());
    Ok((store, config.workspace_dir, config.action_dir))
}

/// Best-effort materialization of a profile's home directory. Logs and swallows
/// filesystem errors — a failed home seed must never fail the RPC (the profile
/// is already persisted; the identity/memory files are lazily re-created on the
/// next select/upsert or read).
fn materialize_home(workspace_dir: &Path, action_dir: &Path, profile: &AgentProfile) {
    if let Err(e) = ensure_profile_home(workspace_dir, action_dir, profile) {
        tracing::warn!(
            profile_id = %profile.id,
            error = %e,
            "[profiles][ops] ensure_profile_home failed (non-fatal)"
        );
    }
}

/// List all persistent profiles and the active id.
pub async fn list() -> Result<Value, String> {
    let request_id = format!("profiles-list-{}", uuid::Uuid::new_v4());
    tracing::debug!(request_id = %request_id, "[profiles][ops] list entry");
    let (store, workspace_dir, action_dir) = store_and_roots().await?;
    let state = store.load().map_err(|e| {
        tracing::debug!(request_id = %request_id, error = %e, "[profiles][ops] list error");
        e
    })?;
    tracing::debug!(
        request_id = %request_id,
        active_profile_id = %state.active_profile_id,
        profile_count = state.profiles.len(),
        "[profiles][ops] list ok"
    );
    Ok(enriched_state_payload(&workspace_dir, &action_dir, &state))
}

/// Select the active profile by id.
pub async fn select(profile_id: &str) -> Result<Value, String> {
    let request_id = format!("profile-select-{}", uuid::Uuid::new_v4());
    tracing::debug!(request_id = %request_id, profile_id, "[profiles][ops] select entry");
    let (store, workspace_dir, action_dir) = store_and_roots().await?;
    let state = store.select(profile_id).map_err(|e| {
        tracing::debug!(request_id = %request_id, profile_id, error = %e, "[profiles][ops] select error");
        e
    })?;
    // Materialize the selected profile's home (covers built-ins, which are only
    // seeded when a user first activates them).
    if let Some(profile) = state
        .profiles
        .iter()
        .find(|p| p.id == state.active_profile_id)
    {
        materialize_home(&workspace_dir, &action_dir, profile);
    }
    tracing::debug!(
        request_id = %request_id,
        profile_id,
        active_profile_id = %state.active_profile_id,
        "[profiles][ops] select ok"
    );
    Ok(enriched_state_payload(&workspace_dir, &action_dir, &state))
}

/// Create or update a profile.
///
/// The profile's `agent_id` is validated against the global
/// [`AgentDefinitionRegistry`](crate::openhuman::agent::harness::AgentDefinitionRegistry)
/// when it is available. When the registry is *not* initialised we **fail
/// closed** for any non-default `agent_id` rather than persist a reference we
/// can't validate — otherwise a startup init-order race would be saved as a
/// permanently-broken profile. The implicit `orchestrator` default (and an empty
/// id, normalised to `orchestrator`) are always valid, mirroring the session
/// builder, so they are admitted without the registry.
pub async fn upsert(profile: AgentProfile) -> Result<Value, String> {
    let request_id = format!("profile-upsert-{}", uuid::Uuid::new_v4());
    let agent_id = profile.agent_id.trim().to_string();
    tracing::debug!(
        request_id = %request_id,
        profile_id = %profile.id,
        agent_id = %agent_id,
        "[profiles][ops] upsert entry"
    );
    match crate::openhuman::agent::harness::AgentDefinitionRegistry::global() {
        Some(registry) => {
            if !agent_id.is_empty() && registry.get(&agent_id).is_none() {
                tracing::debug!(
                    request_id = %request_id,
                    agent_id = %agent_id,
                    "[profiles][ops] upsert unknown_agent"
                );
                return Err(format!("agent definition '{agent_id}' not found"));
            }
        }
        None => {
            // No registry → can only admit the always-valid default agent.
            if !agent_id.is_empty() && agent_id != DEFAULT_AGENT_ID {
                tracing::debug!(
                    request_id = %request_id,
                    agent_id = %agent_id,
                    "[profiles][ops] upsert registry_unavailable fail_closed"
                );
                return Err(format!(
                    "agent definition registry unavailable — cannot validate agent_id '{agent_id}'"
                ));
            }
        }
    }
    let upserted_id = profile.id.clone();
    let (store, workspace_dir, action_dir) = store_and_roots().await?;
    // Keep the previous inline value so SOUL.md is rewritten only when the
    // persona field itself changed. The editor submits the full profile for
    // unrelated settings saves; comparing only against the file would clobber
    // a newer manual edit with the unchanged, stale inline value.
    let normalised_id = super::store::normalise_profile_id(&upserted_id);
    let previous_soul_md = store
        .load()
        .map_err(|e| {
            tracing::debug!(request_id = %request_id, error = %e, "[profiles][ops] upsert preload error");
            e
        })?
        .profiles
        .into_iter()
        .find(|p| p.id == normalised_id)
        .and_then(|p| p.soul_md);
    let state = store.upsert(profile).map_err(|e| {
        tracing::debug!(request_id = %request_id, error = %e, "[profiles][ops] upsert error");
        e
    })?;
    // Materialize the home for the just-upserted profile. The store normalises
    // the id (slugify), so resolve the persisted profile from the returned state
    // rather than trusting the raw input id. Built-ins are seeded on select, not
    // here, matching the spec.
    if let Some(persisted) = state.profiles.iter().find(|p| p.id == normalised_id) {
        // Full home materialization (SOUL.md seed + MEMORY.md + skills/ +
        // dedicated workspace) is for CUSTOM profiles here; built-ins are seeded
        // on `select` (first activation), matching the spec.
        if !persisted.built_in {
            materialize_home(&workspace_dir, &action_dir, persisted);
        }
        // Reconcile an edited persona into the on-disk SOUL.md for EVERY profile,
        // built-in included. `ensure_profile_home` only seeds SOUL.md when absent,
        // and `select` seeds built-in homes on first activation — so a user who
        // selects Default/Research once and later edits its Soul in Settings would
        // otherwise keep a stale `personalities/<id>/SOUL.md` that
        // `resolve_personality_soul` reads before the inline value. The sync is a
        // no-op when `soul_md` is empty/None or unchanged (manual file edits
        // stay authoritative) and creates the home dir if needed. Non-fatal —
        // the profile is already persisted.
        if let Err(e) = super::home::sync_soul_md_on_upsert(
            &workspace_dir,
            persisted,
            previous_soul_md.as_deref(),
        ) {
            tracing::warn!(
                profile_id = %persisted.id,
                error = %e,
                "[profiles][ops] sync_soul_md_on_upsert failed (non-fatal)"
            );
        }
    }
    tracing::debug!(
        request_id = %request_id,
        active_profile_id = %state.active_profile_id,
        profile_count = state.profiles.len(),
        "[profiles][ops] upsert ok"
    );
    Ok(enriched_state_payload(&workspace_dir, &action_dir, &state))
}

/// Delete a custom profile by id.
pub async fn delete(profile_id: &str) -> Result<Value, String> {
    let request_id = format!("profile-delete-{}", uuid::Uuid::new_v4());
    tracing::debug!(request_id = %request_id, profile_id, "[profiles][ops] delete entry");
    let (store, workspace_dir, action_dir) = store_and_roots().await?;
    let state = store.delete(profile_id).map_err(|e| {
        tracing::debug!(request_id = %request_id, profile_id, error = %e, "[profiles][ops] delete error");
        e
    })?;
    tracing::debug!(
        request_id = %request_id,
        profile_id,
        active_profile_id = %state.active_profile_id,
        profile_count = state.profiles.len(),
        "[profiles][ops] delete ok"
    );
    Ok(enriched_state_payload(&workspace_dir, &action_dir, &state))
}

/// The implicit orchestrator agent that requires no registry entry — the
/// built-in default profile uses it and the session builder treats it as always
/// resolvable.
const DEFAULT_AGENT_ID: &str = "orchestrator";

#[cfg(test)]
mod tests {
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
}
