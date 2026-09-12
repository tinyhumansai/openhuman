use super::*;
use crate::openhuman::agent::harness::subagent_runner::SubagentMode;
use std::time::Duration;
use tempfile::TempDir;

fn sample_outcome(output: &str) -> SubagentRunOutcome {
    SubagentRunOutcome {
        agent_id: "researcher".into(),
        task_id: "sub-test-1".into(),
        output: output.to_string(),
        elapsed: Duration::from_millis(120),
        iterations: 3,
        mode: SubagentMode::Typed,
        status: SubagentRunStatus::Completed,
        final_history: Vec::new(),
        usage: Default::default(),
        artifact_paths: Vec::new(),
    }
}

#[test]
fn build_worker_thread_title_collapses_whitespace_and_caps_length() {
    let prompt =
        "  draft\n a very long\tplan that\nrambles ".to_string() + "x".repeat(200).as_str();
    let title = build_worker_thread_title(&prompt);
    assert!(title.starts_with("draft a very long plan"));
    assert!(title.chars().count() <= WORKER_THREAD_TITLE_MAX_CHARS + 1);
    assert!(title.ends_with('…'));
}

#[test]
fn build_worker_thread_title_falls_back_when_empty() {
    assert_eq!(build_worker_thread_title("   \n\t  "), "Worker task");
}

#[test]
fn parameters_schema_advertises_dedicated_thread_flag() {
    let tool = SpawnSubagentTool;
    let schema = tool.parameters_schema();
    let props = schema.get("properties").expect("schema has properties");
    let flag = props
        .get("dedicated_thread")
        .expect("dedicated_thread advertised");
    assert_eq!(flag.get("type").and_then(|v| v.as_str()), Some("boolean"));
    // Must be off by default — workers are an opt-in escape hatch, not
    // a free upgrade for every spawn.
    assert!(schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().all(|s| s.as_str() != Some("dedicated_thread")))
        .unwrap_or(true));
}

#[test]
fn parameters_schema_advertises_optional_model_override() {
    let tool = SpawnSubagentTool;
    let schema = tool.parameters_schema();
    let props = schema.get("properties").expect("schema has properties");
    let model = props.get("model").expect("model override advertised");
    assert_eq!(model.get("type").and_then(|v| v.as_str()), Some("string"));
    assert!(schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().all(|s| s.as_str() != Some("model")))
        .unwrap_or(true));
}

#[test]
fn render_worker_thread_result_carries_machine_readable_envelope() {
    let outcome = sample_outcome("done");
    let rendered = render_worker_thread_result("worker-abc", "researcher", &outcome);
    assert!(rendered.contains("Spawned worker thread `worker-abc`"));
    assert!(rendered.contains("[worker_thread_ref]"));
    assert!(rendered.contains("[/worker_thread_ref]"));
    // The JSON payload between the markers must round-trip.
    let start = rendered.find("[worker_thread_ref]\n").unwrap() + "[worker_thread_ref]\n".len();
    let end = rendered.find("\n[/worker_thread_ref]").unwrap();
    let payload: serde_json::Value =
        serde_json::from_str(&rendered[start..end]).expect("valid json envelope");
    assert_eq!(payload["thread_id"], "worker-abc");
    assert_eq!(payload["label"], "worker");
    assert_eq!(payload["agent_id"], "researcher");
    assert_eq!(payload["task_id"], "sub-test-1");
    assert_eq!(payload["iterations"], 3);
}

#[test]
fn persist_worker_thread_creates_thread_with_tasks_label_and_messages() {
    let temp = TempDir::new().expect("tempdir");
    let outcome = sample_outcome("the answer is 42");
    let thread_id = persist_worker_thread(
        temp.path(),
        "researcher",
        "draft a long research plan",
        &outcome,
    )
    .expect("worker thread persisted");

    assert!(thread_id.starts_with("worker-"));

    let threads = conversations::list_threads(temp.path().to_path_buf()).expect("list threads");
    let worker = threads
        .iter()
        .find(|t| t.id == thread_id)
        .expect("worker thread present");
    assert!(worker.labels.contains(&"tasks".to_string()));
    assert!(worker.title.starts_with("draft a long research plan"));

    let messages =
        conversations::get_messages(temp.path().to_path_buf(), &thread_id).expect("messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].sender, "user");
    assert_eq!(messages[0].content, "draft a long research plan");
    assert_eq!(messages[1].sender, "agent");
    assert_eq!(messages[1].content, "the answer is 42");
    assert_eq!(messages[1].extra_metadata["iterations"], 3);
    assert_eq!(messages[1].extra_metadata["scope"], "worker_thread");
}

#[tokio::test]
async fn missing_agent_id_returns_error() {
    let tool = SpawnSubagentTool;
    let result = tool
        .execute(json!({
            "prompt": "do thing"
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("agent_id"));
}

#[tokio::test]
async fn missing_prompt_returns_error() {
    let tool = SpawnSubagentTool;
    let result = tool
        .execute(json!({
            "agent_id": "researcher"
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("prompt"));
}

#[tokio::test]
async fn no_registry_returns_clear_error() {
    // The global registry has not been initialised in this test.
    let tool = SpawnSubagentTool;
    let result = tool
        .execute(json!({
            "agent_id": "researcher",
            "prompt": "find x",
        }))
        .await
        .unwrap();
    // Either: registry uninitialised → clear init error, OR
    // registry was initialised by a previous test → "no parent context"
    // because we're not running inside an Agent::turn. Both are
    // acceptable: the tool gracefully refuses.
    assert!(result.is_error);
}

#[tokio::test]
async fn unknown_agent_id_lists_available() {
    // Force-init the global registry with builtins.
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let tool = SpawnSubagentTool;
    let result = tool
        .execute(json!({
            "agent_id": "totally_made_up",
            "prompt": "x",
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    let out = result.output();
    // Should list at least one valid built-in.
    assert!(out.contains("code_executor") || out.contains("researcher"));
}

#[test]
fn classify_subagent_failure_reframes_upstream_provider_outages() {
    let msg = SpawnSubagentTool::classify_subagent_failure(
        "provider call failed: all providers/models failed: upstream unavailable",
    );
    assert!(msg.contains("upstream inference unavailable"));
    assert!(msg.contains("NOT a Composio/integration auth issue"));
}

#[tokio::test]
async fn dedicated_thread_flag_no_longer_returns_disabled_error() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let tool = SpawnSubagentTool;
    let result = tool
        .execute(json!({
            "agent_id": "researcher",
            "prompt": "find x",
            "dedicated_thread": true,
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(!result.output().contains("temporarily disabled"));
}

#[tokio::test]
async fn legacy_archetype_alias_is_accepted_for_lookup() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let tool = SpawnSubagentTool;
    let result = tool
        .execute(json!({
            "archetype": "totally_made_up",
            "prompt": "x",
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result
        .output()
        .contains("unknown agent_id 'totally_made_up'"));
}

#[tokio::test]
async fn legacy_archetype_alias_is_normalized_to_agent_id() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let tool = SpawnSubagentTool;
    let result = tool
        .execute(json!({
            "archetype": "researcher",
            "prompt": "research the reusable async default path",
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    // The alias resolved: the call got past argument validation and only
    // failed later, on the missing parent turn.
    assert!(
        !result.output().contains("agent_id is required"),
        "{}",
        result.output()
    );
    assert!(
        result.output().contains("called outside of an agent turn"),
        "{}",
        result.output()
    );
}

/// B40: with no chat thread bound, async-by-default delegation has nowhere
/// to deliver a result, so `spawn_subagent` must self-heal to blocking
/// dispatch rather than forwarding into `spawn_async_subagent`'s
/// thread-less guard — otherwise the guard's own advice ("use
/// `spawn_subagent`") would loop straight back into the guard. Asserted
/// via which tool owns the downstream error.
#[tokio::test]
async fn async_default_self_heals_to_blocking_without_delivery_thread() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let result = SpawnSubagentTool
        .execute(json!({
            "agent_id": "researcher",
            "prompt": "work with no delivery thread",
        }))
        .await
        .unwrap();

    let out = result.output();
    assert!(
        !out.contains("spawn_async_subagent"),
        "thread-less spawn_subagent must not route into the async tool: {out}"
    );
    assert!(
        !out.contains("no parent chat thread"),
        "thread-less spawn_subagent must not hit the async delivery guard: {out}"
    );
    assert!(
        out.contains("spawn_subagent called outside of an agent turn"),
        "expected the blocking path's own error: {out}"
    );
}

#[tokio::test]
async fn integrations_agent_requires_toolkit_argument() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let tool = SpawnSubagentTool;
    let result = tool
        .execute(json!({
            "agent_id": "integrations_agent",
            "prompt": "check gmail",
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    let out = result.output();
    assert!(out.contains("`toolkit` argument is required"));
    assert!(out.contains("currently-connected toolkits"));
}

#[tokio::test]
async fn integrations_agent_rejects_toolkit_outside_allowlist() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let tool = SpawnSubagentTool;
    let toolkit = "totally_not_a_real_toolkit_slug";
    let result = tool
        .execute(json!({
            "agent_id": "integrations_agent",
            "prompt": "check gmail",
            "toolkit": toolkit,
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    let out = result.output();
    assert!(out.contains(&format!(
        "toolkit '{toolkit}' is not in the backend allowlist"
    )));
    assert!(out.contains("Valid toolkits"));
}

// ── #2365: describe_unconnected_state per upstream status ───────

#[test]
fn describe_unconnected_state_initiated_says_oauth_in_progress() {
    let msg = describe_unconnected_state("gmail", Some("INITIATED"));
    assert!(
        msg.contains("OAuth flow in progress"),
        "INITIATED must surface the in-progress wording: {msg}"
    );
    assert!(msg.contains("Connections → 'gmail'"));
    // The legacy "not authorized yet" copy must NOT leak into the
    // pending-OAuth branch — that was the user-perception bug
    // from #2365 (Settings UI showed Gmail connected, agent said
    // "not authorized").
    assert!(
        !msg.contains("has not authorized it yet"),
        "INITIATED must not borrow the truly-disconnected copy: {msg}"
    );
}

#[test]
fn describe_unconnected_state_pending_and_initializing_are_aliased() {
    for status in ["PENDING", "INITIALIZING"] {
        let msg = describe_unconnected_state("gmail", Some(status));
        assert!(
            msg.contains("OAuth flow in progress"),
            "{status} must hit the in-progress branch: {msg}"
        );
    }
}

#[test]
fn describe_unconnected_state_expired_says_reconnect() {
    let msg = describe_unconnected_state("gmail", Some("EXPIRED"));
    assert!(msg.contains("OAuth token has expired"));
    assert!(msg.contains("reconnect 'gmail'"));
    assert!(msg.contains("Connections → 'gmail'"));
    assert!(!msg.contains("Settings → Connections"));
    assert!(!msg.contains("OAuth flow in progress"));
}

#[test]
fn describe_unconnected_state_failed_and_error_route_to_reconnect() {
    for status in ["FAILED", "ERROR"] {
        let msg = describe_unconnected_state("gmail", Some(status));
        let expected = format!("`{status}` state");
        assert!(
            msg.contains(&expected),
            "{status} must be quoted verbatim, not collapsed to a single label: {msg}"
        );
        assert!(msg.contains("reconnect 'gmail'"));
        assert!(msg.contains("Connections → 'gmail'"));
        assert!(!msg.contains("Settings → Connections"));
    }
}

#[test]
fn describe_unconnected_state_failed_and_error_preserve_original_casing() {
    // Mixed-case wire values must round-trip through the FAILED /
    // ERROR branch with their original casing intact — that's the
    // whole point of graycyrus' review feedback.
    let lower_failed = describe_unconnected_state("gmail", Some("failed"));
    assert!(
        lower_failed.contains("`failed` state"),
        "lowercase `failed` must be quoted verbatim: {lower_failed}"
    );
    let mixed_error = describe_unconnected_state("gmail", Some("Error"));
    assert!(
        mixed_error.contains("`Error` state"),
        "mixed-case `Error` must be quoted verbatim: {mixed_error}"
    );
}

#[test]
fn describe_unconnected_state_quotes_unknown_status_verbatim() {
    // Pin three shapes (uppercase / mixed / snake_case) so the
    // verbatim-quoting contract can't silently drift back to
    // echoing the matched (uppercased) value — that was the
    // CodeRabbit finding on #2373.
    for raw in ["DEAUTH_REQUIRED", "needs_relink", "PartialAuthRequired"] {
        let msg = describe_unconnected_state("gmail", Some(raw));
        let expected = format!("`{raw}`");
        assert!(
            msg.contains(&expected),
            "unknown status `{raw}` must be quoted verbatim (not its uppercased form): {msg}"
        );
        assert!(msg.contains("Connections → 'gmail'"));
        assert!(!msg.contains("Settings → Connections"));
    }
}

#[test]
fn describe_unconnected_state_quotes_unknown_status_after_trimming_whitespace() {
    // Whitespace-only / blank statuses must NOT hit the
    // unknown-status branch — they collapse to the
    // truly-disconnected legacy copy via the `filter(|s|
    // !s.is_empty())` guard in `describe_unconnected_state`.
    let blank = describe_unconnected_state("gmail", Some("   "));
    assert!(
        blank.contains("has not authorized it yet"),
        "whitespace-only status must collapse to legacy None branch: {blank}"
    );
    // A real status with surrounding whitespace is quoted with
    // the whitespace trimmed (not preserved verbatim — triage
    // would not want padded backticks).
    let padded = describe_unconnected_state("gmail", Some("  DeauthRequired  "));
    assert!(
        padded.contains("`DeauthRequired`"),
        "trimmed status must be quoted in original casing: {padded}"
    );
}

#[test]
fn describe_unconnected_state_none_is_truly_disconnected() {
    let msg = describe_unconnected_state("gmail", None);
    assert!(
        msg.contains("has not authorized it yet"),
        "None must hit the legacy never-connected copy: {msg}"
    );
    assert!(msg.contains("Connections → 'gmail'"));
}

#[test]
fn describe_unconnected_state_status_match_is_case_insensitive() {
    // The status string flows in from Composio's wire format; we
    // can't assume casing. The classifier must normalise.
    let initiated = describe_unconnected_state("gmail", Some("initiated"));
    assert!(initiated.contains("OAuth flow in progress"));
    let expired = describe_unconnected_state("gmail", Some("Expired"));
    assert!(expired.contains("OAuth token has expired"));
}
