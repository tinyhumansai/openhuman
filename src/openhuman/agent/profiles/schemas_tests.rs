use super::*;
use crate::openhuman::agent::profiles::DEFAULT_PROFILE_ID;
use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;
use serde_json::json;

#[test]
fn controller_schema_inventory_is_stable() {
    let schemas = all_controller_schemas();
    let functions: Vec<_> = schemas.iter().map(|schema| schema.function).collect();
    assert_eq!(functions, vec!["list", "select", "upsert", "delete"]);
    assert_eq!(schemas.len(), all_registered_controllers().len());
    assert!(schemas.iter().all(|s| s.namespace == "profiles"));
}

#[test]
fn unknown_function_falls_back() {
    let unknown = schemas("nope");
    assert_eq!(unknown.function, "unknown");
    assert_eq!(unknown.outputs[0].name, "error");
}

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
            Some(value) => unsafe {
                std::env::set_var("OPENHUMAN_WORKSPACE", value);
            },
            None => unsafe {
                std::env::remove_var("OPENHUMAN_WORKSPACE");
            },
        }
    }
}

#[tokio::test]
async fn profile_handlers_persist_and_return_profile_state() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let _env = WorkspaceEnvGuard::set(temp.path());

    let upserted = handle_profile_upsert(Map::from_iter([(
        "profile".into(),
        json!({
            "id": "writer",
            "name": "Writer",
            "description": "Draft concise copy",
            "agentId": "orchestrator",
            "modelOverride": "agentic-v1",
            "temperature": 0.2,
            "systemPromptSuffix": "Use a crisp tone.",
            "allowedTools": ["todo"],
            "memorySources": ["slack-eng"],
            "allowedSkills": ["deep-research"],
            "includeAgentConversations": false,
            "builtIn": false,
        }),
    )]))
    .await
    .expect("profile upsert");
    assert_eq!(upserted["activeProfileId"], DEFAULT_PROFILE_ID);
    let writer = upserted["profiles"]
        .as_array()
        .expect("profiles array")
        .iter()
        .find(|profile| profile["id"] == "writer")
        .expect("writer profile present");
    assert_eq!(writer["memorySources"], json!(["slack-eng"]));
    assert_eq!(writer["allowedSkills"], json!(["deep-research"]));
    assert_eq!(writer["includeAgentConversations"], json!(false));

    let selected = handle_profile_select(Map::from_iter([(
        "profile_id".into(),
        Value::String("writer".into()),
    )]))
    .await
    .expect("profile select");
    assert_eq!(selected["activeProfileId"], "writer");

    let listed = handle_profiles_list(Map::new())
        .await
        .expect("profiles list");
    assert_eq!(listed["activeProfileId"], "writer");

    let deleted = handle_profile_delete(Map::from_iter([(
        "profile_id".into(),
        Value::String("writer".into()),
    )]))
    .await
    .expect("profile delete");
    assert_eq!(deleted["activeProfileId"], DEFAULT_PROFILE_ID);
}

/// Resolve the enriched `soulMdFile` absolute path for `profile_id` from a
/// profiles-state payload (present once the home's SOUL.md exists on disk).
fn soul_md_file(payload: &Value, profile_id: &str) -> Option<String> {
    payload["profiles"]
        .as_array()?
        .iter()
        .find(|p| p["id"] == profile_id)?
        .get("soulMdFile")?
        .as_str()
        .map(str::to_string)
}

#[tokio::test]
async fn built_in_profile_soul_edit_syncs_to_disk() {
    // Regression (PR #5118 review, Codex): select() seeds a built-in's home on
    // first activation, so a later Soul edit through the editor must reconcile
    // the on-disk SOUL.md — the sync path must NOT be gated on `!built_in`.
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let _env = WorkspaceEnvGuard::set(temp.path());

    const PROFILE_ID: &str = "reasoning";
    // Seed a non-default built-in home the way first activation does; the
    // unedited Default intentionally keeps using the legacy root SOUL.md.
    // enriched payload advertises the resolved SOUL.md path once it exists.
    let selected = handle_profile_select(Map::from_iter([(
        "profile_id".into(),
        Value::String(PROFILE_ID.into()),
    )]))
    .await
    .expect("select built-in");
    let soul_path = soul_md_file(&selected, PROFILE_ID).expect("select seeds built-in SOUL.md");

    // User later edits the built-in's Soul in Settings.
    handle_profile_upsert(Map::from_iter([(
        "profile".into(),
        json!({
            "id": PROFILE_ID,
            "name": "Reasoning",
            "description": "",
            "agentId": "orchestrator",
            "soulMd": "Edited built-in persona.",
            "builtIn": true,
        }),
    )]))
    .await
    .expect("upsert default with edited soul");

    assert_eq!(
        std::fs::read_to_string(&soul_path).unwrap(),
        "Edited built-in persona.\n",
        "editing a built-in's soul must overwrite its seeded SOUL.md"
    );
}

#[tokio::test]
async fn built_in_profile_empty_soul_leaves_file_untouched() {
    // The sync is a no-op when soulMd is empty/None, so a user's manual edit
    // to a built-in's SOUL.md stays authoritative.
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let _env = WorkspaceEnvGuard::set(temp.path());

    const PROFILE_ID: &str = "reasoning";
    let selected = handle_profile_select(Map::from_iter([(
        "profile_id".into(),
        Value::String(PROFILE_ID.into()),
    )]))
    .await
    .expect("select built-in");
    let soul_path = soul_md_file(&selected, PROFILE_ID).expect("select seeds built-in SOUL.md");
    std::fs::write(&soul_path, "MANUAL EDIT").unwrap();

    // Upsert with no soulMd — must not touch the manually edited file.
    handle_profile_upsert(Map::from_iter([(
        "profile".into(),
        json!({
            "id": PROFILE_ID,
            "name": "Reasoning",
            "description": "",
            "agentId": "orchestrator",
            "builtIn": true,
        }),
    )]))
    .await
    .expect("upsert default without soul");

    assert_eq!(
        std::fs::read_to_string(&soul_path).unwrap(),
        "MANUAL EDIT",
        "empty soulMd must leave a manually edited built-in SOUL.md untouched"
    );
}

#[tokio::test]
async fn profile_upsert_rejects_unknown_registered_agent_id() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global_builtins();

    let err = handle_profile_upsert(Map::from_iter([(
        "profile".into(),
        json!({
            "id": "bad",
            "name": "Bad",
            "description": "",
            "agentId": "__missing_agent__",
            "builtIn": false,
        }),
    )]))
    .await
    .expect_err("unknown agent should fail before store write");
    assert!(err.contains("agent definition"), "err: {err}");
}
