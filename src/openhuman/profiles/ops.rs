//! Business logic for the `profiles` RPC surface.
//!
//! Per the OpenHuman domain contract, controller handlers in [`super::schemas`]
//! stay thin (deserialize + delegate) and the real work — config loading,
//! `agent_id` validation, and store mutation — lives here, returning the JSON
//! payload the controller emits. The persistence itself is owned by
//! [`AgentProfileStore`](super::store::AgentProfileStore).

use serde_json::Value;

use super::store::AgentProfileStore;
use super::types::{AgentProfile, AgentProfilesState};
use crate::openhuman::config::rpc as config_rpc;

/// Shape the `{ profiles, activeProfileId }` payload every profiles RPC returns.
fn state_payload(state: &AgentProfilesState) -> Value {
    serde_json::json!({
        "profiles": state.profiles,
        "activeProfileId": state.active_profile_id,
    })
}

/// Resolve the workspace-scoped profile store from the loaded config.
async fn store() -> Result<AgentProfileStore, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    Ok(AgentProfileStore::new(config.workspace_dir))
}

/// List all persistent profiles and the active id.
pub async fn list() -> Result<Value, String> {
    let request_id = format!("profiles-list-{}", uuid::Uuid::new_v4());
    tracing::debug!(request_id = %request_id, "[profiles][ops] list entry");
    let state = store().await?.load().map_err(|e| {
        tracing::debug!(request_id = %request_id, error = %e, "[profiles][ops] list error");
        e
    })?;
    tracing::debug!(
        request_id = %request_id,
        active_profile_id = %state.active_profile_id,
        profile_count = state.profiles.len(),
        "[profiles][ops] list ok"
    );
    Ok(state_payload(&state))
}

/// Select the active profile by id.
pub async fn select(profile_id: &str) -> Result<Value, String> {
    let request_id = format!("profile-select-{}", uuid::Uuid::new_v4());
    tracing::debug!(request_id = %request_id, profile_id, "[profiles][ops] select entry");
    let state = store().await?.select(profile_id).map_err(|e| {
        tracing::debug!(request_id = %request_id, profile_id, error = %e, "[profiles][ops] select error");
        e
    })?;
    tracing::debug!(
        request_id = %request_id,
        profile_id,
        active_profile_id = %state.active_profile_id,
        "[profiles][ops] select ok"
    );
    Ok(state_payload(&state))
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
    let state = store().await?.upsert(profile).map_err(|e| {
        tracing::debug!(request_id = %request_id, error = %e, "[profiles][ops] upsert error");
        e
    })?;
    tracing::debug!(
        request_id = %request_id,
        active_profile_id = %state.active_profile_id,
        profile_count = state.profiles.len(),
        "[profiles][ops] upsert ok"
    );
    Ok(state_payload(&state))
}

/// Delete a custom profile by id.
pub async fn delete(profile_id: &str) -> Result<Value, String> {
    let request_id = format!("profile-delete-{}", uuid::Uuid::new_v4());
    tracing::debug!(request_id = %request_id, profile_id, "[profiles][ops] delete entry");
    let state = store().await?.delete(profile_id).map_err(|e| {
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
    Ok(state_payload(&state))
}

/// The implicit orchestrator agent that requires no registry entry — the
/// built-in default profile uses it and the session builder treats it as always
/// resolvable.
const DEFAULT_AGENT_ID: &str = "orchestrator";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;
    use crate::openhuman::profiles::DEFAULT_PROFILE_ID;
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
