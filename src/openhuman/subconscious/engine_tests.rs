use super::*;

struct EnvVarGuard {
    key: &'static str,
    old: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &std::path::Path) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, path);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn test_tasks() -> Vec<SubconsciousTask> {
    vec![
        SubconsciousTask {
            id: "t1".into(),
            title: "Check email".into(),
            source: TaskSource::User,
            recurrence: TaskRecurrence::Cron("0 8 * * *".into()),
            enabled: true,
            last_run_at: None,
            next_run_at: None,
            completed: false,
            created_at: 0.0,
        },
        SubconsciousTask {
            id: "t2".into(),
            title: "Monitor skills".into(),
            source: TaskSource::System,
            recurrence: TaskRecurrence::Pending,
            enabled: true,
            last_run_at: None,
            next_run_at: None,
            completed: false,
            created_at: 0.0,
        },
    ]
}

#[tokio::test]
async fn tick_skips_unavailable_provider_without_activity_log_spam() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let _workspace = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path());
    let config = Config::load_or_init().await.expect("load test config");
    let engine = SubconsciousEngine::from_heartbeat_config(
        &config.heartbeat,
        config.workspace_dir.clone(),
        None,
    );

    let result = engine.tick().await.expect("tick should skip cleanly");

    assert!(result.evaluations.is_empty());
    let logs = store::with_connection(&config.workspace_dir, |conn| {
        store::list_log_entries(conn, None, 20)
    })
    .expect("list logs");
    assert!(
        logs.is_empty(),
        "provider skip must not append per-task failure log entries"
    );

    let status = engine.status().await;
    assert_eq!(status.consecutive_failures, 1);
    assert!(!status.provider_available);
    assert!(status
        .provider_unavailable_reason
        .as_deref()
        .unwrap_or_default()
        .contains("Sign in"));

    let _second = engine.tick().await.expect("repeat skip should be clean");
    let logs = store::with_connection(&config.workspace_dir, |conn| {
        store::list_log_entries(conn, None, 20)
    })
    .expect("list logs after repeat");
    assert!(logs.is_empty(), "repeat skips must not spam activity log");
}

#[test]
fn local_subconscious_provider_is_available() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.subconscious_provider = Some("ollama:qwen2.5:0.5b".into());

    assert!(subconscious_provider_unavailable_reason(&config).is_none());
}

#[test]
fn local_subconscious_route_preserves_ollama_model() {
    let mut config = Config::default();
    config.subconscious_provider = Some("ollama:qwen2.5:0.5b".into());

    assert_eq!(
        resolve_subconscious_route(&config),
        SubconsciousProviderRoute::LocalOllama {
            model: "qwen2.5:0.5b".into(),
        }
    );
}

#[test]
fn local_subconscious_provider_does_not_require_legacy_endpoint() {
    let mut config = Config::default();
    config.subconscious_provider = Some("ollama:qwen2.5:0.5b".into());
    config.memory_tree.llm_summariser_endpoint = None;

    assert!(subconscious_provider_unavailable_reason(&config).is_none());
}

#[test]
fn openhuman_subconscious_alias_uses_cloud_route() {
    let mut config = Config::default();
    config.subconscious_provider = Some("openhuman:summarization".into());

    assert_eq!(
        resolve_subconscious_route(&config),
        SubconsciousProviderRoute::OpenHumanCloud
    );
}

#[test]
fn explicit_subconscious_provider_uses_other_route() {
    let mut config = Config::default();
    config.subconscious_provider = Some("custom-provider".into());

    assert_eq!(
        resolve_subconscious_route(&config),
        SubconsciousProviderRoute::Other("custom-provider".into())
    );
    assert!(subconscious_provider_unavailable_reason(&config).is_none());
}

#[test]
fn parse_evaluation_response() {
    let json = r#"{"evaluations": [
        {"task_id": "t1", "decision": "act", "reason": "3 new urgent emails"},
        {"task_id": "t2", "decision": "noop", "reason": "All skills healthy"}
    ]}"#;
    let (evals, drafts) = parse_response(json, &test_tasks());
    assert_eq!(evals.len(), 2);
    assert_eq!(evals[0].decision, TickDecision::Act);
    assert_eq!(evals[1].decision, TickDecision::Noop);
    assert!(drafts.is_empty());
}

#[test]
fn parse_evaluation_bare_array() {
    let json = r#"[
        {"task_id": "t1", "decision": "escalate", "reason": "Deadline conflict"}
    ]"#;
    let (evals, drafts) = parse_response(json, &test_tasks());
    assert_eq!(evals.len(), 1);
    assert_eq!(evals[0].decision, TickDecision::Escalate);
    assert!(drafts.is_empty());
}

#[test]
fn parse_evaluation_in_markdown() {
    let json = "```json\n{\"evaluations\": [{\"task_id\": \"t1\", \"decision\": \"act\", \"reason\": \"Found items\"}]}\n```";
    let (evals, _) = parse_response(json, &test_tasks());
    assert_eq!(evals.len(), 1);
    assert_eq!(evals[0].decision, TickDecision::Act);
}

#[test]
fn parse_evaluation_garbage_falls_back_to_noop() {
    let (evals, drafts) = parse_response("Not JSON at all", &test_tasks());
    assert_eq!(evals.len(), 2);
    assert!(evals.iter().all(|e| e.decision == TickDecision::Noop));
    assert!(drafts.is_empty());
}

#[test]
fn parse_response_extracts_reflections() {
    let json = r#"{
        "evaluations": [{"task_id": "t1", "decision": "noop", "reason": "nothing"}],
        "reflections": [
            {
                "kind": "hotness_spike",
                "body": "Phoenix surge",
                "disposition": "notify",
                "proposed_action": "Pull mentions",
                "source_refs": ["entity:phoenix"]
            },
            {
                "kind": "daily_digest",
                "body": "New digest",
                "disposition": "observe"
            }
        ]
    }"#;
    let (evals, drafts) = parse_response(json, &test_tasks());
    assert_eq!(evals.len(), 1);
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts[0].body, "Phoenix surge");
    assert_eq!(drafts[1].body, "New digest");
}

#[test]
fn parse_response_handles_only_reflections() {
    // LLM emitted reflections but no per-task evaluations.
    let json = r#"{
        "evaluations": [],
        "reflections": [
            {"kind": "risk", "body": "Concerning pattern", "disposition": "notify"}
        ]
    }"#;
    let (evals, drafts) = parse_response(json, &test_tasks());
    // Tasks default to Noop so the existing tick loop still updates log entries.
    assert_eq!(evals.len(), 2);
    assert!(evals.iter().all(|e| e.decision == TickDecision::Noop));
    assert_eq!(drafts.len(), 1);
}

#[test]
fn extract_json_object() {
    assert_eq!(extract_json(r#"{"key": "val"}"#), r#"{"key": "val"}"#);
}

#[test]
fn extract_json_from_text() {
    let input = "Here's the result: {\"evaluations\": []} done.";
    assert!(extract_json(input).starts_with('{'));
    assert!(extract_json(input).ends_with('}'));
}
