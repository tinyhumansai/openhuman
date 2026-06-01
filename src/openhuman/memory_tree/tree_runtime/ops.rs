//! RPC operation wrappers for the tree summarizer.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::memory_tree::tree_runtime::{engine, store, types::*};
use crate::rpc::RpcOutcome;

/// Append raw content to the ingestion buffer.
pub async fn tree_summarizer_ingest(
    config: &Config,
    namespace: &str,
    content: &str,
    timestamp: Option<DateTime<Utc>>,
    metadata: Option<&Value>,
) -> Result<RpcOutcome<Value>, String> {
    store::validate_namespace(namespace)?;
    if content.trim().is_empty() {
        return Err("content must not be empty".to_string());
    }

    let ts = timestamp.unwrap_or_else(Utc::now);
    let path = store::buffer_write(config, namespace.trim(), content, &ts, metadata)
        .map_err(|e| format!("buffer write failed: {e}"))?;

    Ok(RpcOutcome::single_log(
        json!({
            "buffered": true,
            "namespace": namespace.trim(),
            "timestamp": ts.to_rfc3339(),
            "tokens": estimate_tokens(content),
            "path": path.display().to_string(),
            "has_metadata": metadata.is_some(),
        }),
        format!("content buffered for namespace '{}'", namespace.trim()),
    ))
}

/// Trigger the summarization job for a namespace (drain buffer + summarize + propagate).
pub async fn tree_summarizer_run(
    config: &Config,
    namespace: &str,
) -> Result<RpcOutcome<Value>, String> {
    store::validate_namespace(namespace)?;

    let (provider, model) = create_provider(config)?;
    let ts = Utc::now();

    match engine::run_summarization(config, provider.as_ref(), &model, namespace.trim(), ts).await {
        Ok(Some(node)) => Ok(RpcOutcome::single_log(
            serde_json::to_value(&node).map_err(|e| e.to_string())?,
            format!(
                "summarization completed for '{}': node {} ({} tokens)",
                namespace.trim(),
                node.node_id,
                node.token_count
            ),
        )),
        Ok(None) => Ok(RpcOutcome::single_log(
            json!({ "skipped": true, "reason": "no buffered data" }),
            format!(
                "summarization skipped for '{}': no buffered data",
                namespace.trim()
            ),
        )),
        Err(e) => Err(format!("summarization failed: {e:#}")),
    }
}

/// Query the tree at a specific node or level.
pub async fn tree_summarizer_query(
    config: &Config,
    namespace: &str,
    node_id: Option<&str>,
) -> Result<RpcOutcome<Value>, String> {
    store::validate_namespace(namespace)?;

    let target_id = node_id.unwrap_or("root");
    store::validate_node_id(target_id)?;

    let node = store::read_node(config, namespace.trim(), target_id)
        .map_err(|e| format!("read node: {e}"))?
        .ok_or_else(|| {
            format!(
                "node '{}' not found in namespace '{}'",
                target_id,
                namespace.trim()
            )
        })?;

    let children = store::read_children(config, namespace.trim(), target_id)
        .map_err(|e| format!("read children: {e}"))?;

    let result = QueryResult { node, children };
    Ok(RpcOutcome::single_log(
        serde_json::to_value(&result).map_err(|e| e.to_string())?,
        format!(
            "queried node '{}' in namespace '{}'",
            target_id,
            namespace.trim()
        ),
    ))
}

/// Get tree status/metadata for a namespace.
pub async fn tree_summarizer_status(
    config: &Config,
    namespace: &str,
) -> Result<RpcOutcome<Value>, String> {
    store::validate_namespace(namespace)?;

    let status =
        store::get_tree_status(config, namespace.trim()).map_err(|e| format!("get status: {e}"))?;

    Ok(RpcOutcome::single_log(
        serde_json::to_value(&status).map_err(|e| e.to_string())?,
        format!("tree status for namespace '{}'", namespace.trim()),
    ))
}

/// Rebuild the entire tree from hour leaves (background task).
pub async fn tree_summarizer_rebuild(
    config: &Config,
    namespace: &str,
) -> Result<RpcOutcome<Value>, String> {
    store::validate_namespace(namespace)?;

    let (provider, model) = create_provider(config)?;

    let status = engine::rebuild_tree(config, provider.as_ref(), &model, namespace.trim())
        .await
        .map_err(|e| format!("rebuild failed: {e:#}"))?;

    Ok(RpcOutcome::single_log(
        serde_json::to_value(&status).map_err(|e| e.to_string())?,
        format!(
            "tree rebuilt for '{}': {} nodes",
            namespace.trim(),
            status.total_nodes
        ),
    ))
}

// ── Helper ─────────────────────────────────────────────────────────────

/// Build the (provider, model) pair the summarizer runs on (#002 FR-007).
///
/// Historically this hard-required local AI ("private + offline"), which left
/// "Build Summary Trees" dead for cloud-only setups (Tencent/OpenRouter with
/// no local Ollama). It now falls back to the **configured cloud chat
/// provider** for the summarization role when local AI is off, returning that
/// provider's model id alongside it so the engine targets the right model
/// (the engine no longer assumes the local model id). The UI shows a
/// Resolve the summarization provider.
///
/// Priority:
/// 1. Local Ollama when `local_ai.runtime_enabled = true`.
/// 2. Cloud via `create_chat_provider` when
///    `memory_tree.cloud_summarization_opt_in = true` — the user has
///    explicitly acknowledged that memory summaries will be sent to an
///    external provider.
/// 3. Error otherwise — "Build Summary Trees" is local-only by default;
///    the user must opt in to cloud summarization in Settings → AI → Memory.
fn create_provider(
    config: &Config,
) -> Result<
    (
        Box<dyn crate::openhuman::inference::provider::traits::Provider>,
        String,
    ),
    String,
> {
    if config.local_ai.runtime_enabled {
        // Local path: Ollama + the user's local chat model.
        let provider = create_local_ai_provider(config)?;
        return Ok((provider, config.local_ai.chat_model_id.clone()));
    }

    if !config.memory_tree.cloud_summarization_opt_in {
        return Err("no summarization provider — enable local AI, or enable \
             cloud summarization in Settings → AI → Memory"
            .to_string());
    }

    // Cloud path — user has explicitly opted in. Build the configured
    // provider for the summarization role (`memory_provider` hint).
    crate::openhuman::inference::provider::factory::create_chat_provider("summarization", config)
        .map_err(|e| format!("tree summarizer: failed to build cloud provider: {e:#}"))
}

/// Whether a summarization provider can be resolved for "Build Summary Trees"
/// under the current config — the single source of truth the memory doctor
/// reuses so its `summary_tree` stage matches the runtime path (#002 FR-007).
///
/// Routes through [`create_provider`] (the SAME resolver the runtime uses):
/// - local AI enabled ⇒ available (local Ollama path).
/// - local AI off + `memory_tree.cloud_summarization_opt_in = true` ⇒
///   available iff the configured summarization-role provider resolves.
/// - local AI off + opt-in `false` (default) ⇒ unavailable — explicit
///   consent required before routing workspace memory summaries to a cloud
///   provider. Enable in Settings → AI → Memory.
///
/// The provider built for the `Ok` check is dropped — construction is cheap
/// (no network) and confirming by build beats guessing.
pub fn summarizer_available(config: &Config) -> (bool, &'static str) {
    let local = config.local_ai.runtime_enabled;
    match create_provider(config) {
        Ok(_) if local => (
            true,
            "local AI enabled — Build Summary Trees runs on the local model",
        ),
        Ok(_) => (
            true,
            "local AI off — Build Summary Trees runs on the configured cloud provider",
        ),
        Err(_) => (
            false,
            "no summarization provider available — enable local AI or configure a cloud provider in Settings → AI",
        ),
    }
}

/// Create a provider backed by the local Ollama instance for summarization,
/// wrapped in `ReliableProvider` for retry/backoff on transient failures.
fn create_local_ai_provider(
    config: &Config,
) -> Result<Box<dyn crate::openhuman::inference::provider::traits::Provider>, String> {
    use crate::openhuman::inference::local::OLLAMA_BASE_URL;
    use crate::openhuman::inference::provider::compatible::{AuthStyle, OpenAiCompatibleProvider};
    use crate::openhuman::inference::provider::reliable::ReliableProvider;

    let base_url = format!("{}/v1", OLLAMA_BASE_URL);
    let inner = OpenAiCompatibleProvider::new_no_responses_fallback(
        "ollama-local",
        &base_url,
        None,
        AuthStyle::None,
    );

    let providers: Vec<(
        String,
        Box<dyn crate::openhuman::inference::provider::traits::Provider>,
    )> = vec![("ollama-local".to_string(), Box::new(inner))];
    let reliable = ReliableProvider::new(
        providers,
        config.reliability.provider_retries,
        config.reliability.provider_backoff_ms,
    );

    tracing::debug!(
        "[tree_summarizer] using local Ollama provider at {} with model '{}'",
        base_url,
        config.local_ai.chat_model_id
    );

    Ok(Box::new(reliable))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn rfc3339_z(ts: DateTime<Utc>) -> String {
        ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    fn config_in_tempdir() -> (TempDir, Config) {
        let tmp = TempDir::new().expect("tempdir");
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    fn test_node(
        namespace: &str,
        node_id: &str,
        summary: &str,
        created_at: DateTime<Utc>,
        child_count: u32,
    ) -> TreeNode {
        TreeNode {
            node_id: node_id.to_string(),
            namespace: namespace.to_string(),
            level: level_from_node_id(node_id),
            parent_id: derive_parent_id(node_id),
            summary: summary.to_string(),
            token_count: estimate_tokens(summary),
            child_count,
            created_at,
            updated_at: created_at,
            metadata: None,
        }
    }

    #[test]
    fn create_provider_uses_local_model_when_local_ai_enabled() {
        // #002 FR-007: local path returns the user's local chat model.
        let mut cfg = Config::default();
        cfg.local_ai.runtime_enabled = true;
        cfg.local_ai.chat_model_id = "qwen2.5:7b".to_string();
        let (_provider, model) = create_provider(&cfg).expect("local provider should build");
        assert_eq!(model, "qwen2.5:7b");
    }

    #[test]
    fn create_provider_errors_without_cloud_opt_in() {
        // By default, cloud summarization is off — memory summaries are
        // sensitive, so an explicit opt-in is required before routing them to
        // an external provider.
        let mut cfg = Config::default();
        cfg.local_ai.runtime_enabled = false;
        // cloud_summarization_opt_in defaults to false
        match create_provider(&cfg) {
            Err(e) => assert!(
                e.contains("no summarization provider"),
                "unexpected error: {e}"
            ),
            Ok(_) => panic!("expected error without cloud opt-in"),
        }
    }

    #[test]
    fn create_provider_uses_cloud_when_opted_in_and_local_ai_off() {
        // #002 FR-007: with explicit opt-in Build Summary Trees uses the
        // configured cloud provider when local AI is disabled.
        let mut cfg = Config::default();
        cfg.local_ai.runtime_enabled = false;
        cfg.memory_tree.cloud_summarization_opt_in = true;
        let (_provider, model) =
            create_provider(&cfg).expect("cloud fallback should build when opted in");
        assert!(
            !model.trim().is_empty(),
            "cloud fallback must resolve a model"
        );
    }

    #[test]
    fn create_local_ai_provider_uses_ollama_local_label() {
        let mut cfg = Config::default();
        cfg.local_ai.runtime_enabled = true;
        let provider = create_local_ai_provider(&cfg).expect("provider");
        let _ = provider;
    }

    #[tokio::test]
    async fn tree_summarizer_ingest_rejects_blank_content() {
        let (_tmp, cfg) = config_in_tempdir();
        let err = tree_summarizer_ingest(&cfg, "team", "   ", None, None)
            .await
            .expect_err("blank content should be rejected");
        assert!(err.contains("content must not be empty"));
    }

    #[tokio::test]
    async fn tree_summarizer_ingest_writes_buffer_and_reports_metadata() {
        let (_tmp, cfg) = config_in_tempdir();
        let ts = chrono::Utc
            .with_ymd_and_hms(2026, 5, 24, 12, 30, 0)
            .unwrap();
        let meta = json!({"source": "unit-test"});
        let outcome =
            tree_summarizer_ingest(&cfg, "Team / Notes", "hello world", Some(ts), Some(&meta))
                .await
                .expect("ingest should succeed");

        assert_eq!(
            outcome.logs,
            vec!["content buffered for namespace 'Team / Notes'".to_string()]
        );
        assert_eq!(outcome.value["buffered"], true);
        assert_eq!(outcome.value["namespace"], "Team / Notes");
        assert_eq!(
            outcome.value["tokens"],
            json!(estimate_tokens("hello world"))
        );
        assert_eq!(outcome.value["has_metadata"], true);

        let path = outcome.value["path"]
            .as_str()
            .expect("path string in response");
        let written = std::fs::read_to_string(path).expect("buffer file should exist");
        assert!(written.contains("hello world"));
        assert!(written.contains("\"source\":\"unit-test\""));
    }

    #[tokio::test]
    async fn tree_summarizer_status_reports_empty_tree_defaults() {
        let (_tmp, cfg) = config_in_tempdir();
        let outcome = tree_summarizer_status(&cfg, "fresh-ns")
            .await
            .expect("status on fresh namespace");
        assert_eq!(
            outcome.logs,
            vec!["tree status for namespace 'fresh-ns'".to_string()]
        );
        assert_eq!(outcome.value["namespace"], "fresh-ns");
        assert_eq!(outcome.value["total_nodes"], 0);
        assert_eq!(outcome.value["depth"], 0);
    }

    #[tokio::test]
    async fn tree_summarizer_query_errors_when_node_is_missing() {
        let (_tmp, cfg) = config_in_tempdir();
        let err = tree_summarizer_query(&cfg, "fresh-ns", Some("root"))
            .await
            .expect_err("missing node should error");
        assert!(err.contains("node 'root' not found in namespace 'fresh-ns'"));
    }

    #[tokio::test]
    async fn tree_summarizer_query_returns_node_and_children() {
        let (_tmp, cfg) = config_in_tempdir();
        let ts = chrono::Utc
            .with_ymd_and_hms(2026, 5, 24, 12, 30, 0)
            .unwrap();
        let root = test_node("team", "root", "root summary", ts, 1);
        let year = test_node("team", "2026", "year summary", ts, 1);
        store::write_node(&cfg, &root).expect("write root");
        store::write_node(&cfg, &year).expect("write year");

        let outcome = tree_summarizer_query(&cfg, "team", None)
            .await
            .expect("query should succeed");

        assert_eq!(
            outcome.logs,
            vec!["queried node 'root' in namespace 'team'"]
        );
        assert_eq!(outcome.value["node"]["node_id"], "root");
        assert_eq!(outcome.value["node"]["summary"], "root summary");
        assert_eq!(
            outcome.value["children"],
            json!([{
                "node_id": "2026",
                "namespace": "team",
                "level": "year",
                "parent_id": "root",
                "summary": "year summary",
                "token_count": estimate_tokens("year summary"),
                "child_count": 1,
                "created_at": rfc3339_z(ts),
                "updated_at": rfc3339_z(ts)
            }])
        );
    }

    #[tokio::test]
    async fn tree_summarizer_status_reports_populated_tree_details() {
        let (_tmp, cfg) = config_in_tempdir();
        let early = chrono::Utc.with_ymd_and_hms(2026, 5, 24, 8, 0, 0).unwrap();
        let late = chrono::Utc.with_ymd_and_hms(2026, 5, 24, 17, 0, 0).unwrap();
        for node in [
            test_node("team", "root", "root summary", early, 1),
            test_node("team", "2026", "year summary", early, 1),
            test_node("team", "2026/05", "month summary", early, 1),
            test_node("team", "2026/05/24", "day summary", early, 2),
            test_node("team", "2026/05/24/08", "hour one", early, 0),
            test_node("team", "2026/05/24/17", "hour two", late, 0),
        ] {
            store::write_node(&cfg, &node).expect("write test node");
        }

        let outcome = tree_summarizer_status(&cfg, "team")
            .await
            .expect("status should succeed");

        assert_eq!(outcome.logs, vec!["tree status for namespace 'team'"]);
        assert_eq!(outcome.value["namespace"], "team");
        assert_eq!(outcome.value["total_nodes"], 6);
        assert_eq!(outcome.value["depth"], 5);
        assert_eq!(outcome.value["oldest_entry"], rfc3339_z(early));
        assert_eq!(outcome.value["newest_entry"], rfc3339_z(late));
        assert_eq!(outcome.value["last_run_at"], Value::Null);
    }

    #[tokio::test]
    async fn tree_summarizer_run_skips_when_buffer_is_empty() {
        let (_tmp, mut cfg) = config_in_tempdir();
        cfg.local_ai.runtime_enabled = true;

        let outcome = tree_summarizer_run(&cfg, "team")
            .await
            .expect("empty buffer should skip");

        assert_eq!(
            outcome.logs,
            vec!["summarization skipped for 'team': no buffered data"]
        );
        assert_eq!(
            outcome.value,
            json!({ "skipped": true, "reason": "no buffered data" })
        );
        assert!(
            !store::buffer_dir(&cfg, "team").exists(),
            "skip path should not create a buffer directory"
        );
    }

    #[tokio::test]
    async fn tree_summarizer_run_skips_cleanly_with_cloud_fallback_and_empty_buffer() {
        // #002 FR-007 (Gray review updated): with local AI off AND explicit cloud
        // opt-in, run/rebuild do not hard-error on the provider precondition.
        // With an empty buffer, `run` reports the normal "no buffered data" skip.
        let (_tmp, mut cfg) = config_in_tempdir();
        cfg.local_ai.runtime_enabled = false;
        cfg.memory_tree.cloud_summarization_opt_in = true;

        let outcome = tree_summarizer_run(&cfg, "team")
            .await
            .expect("run should not error on the provider precondition when opted in");
        assert_eq!(
            outcome.value,
            json!({ "skipped": true, "reason": "no buffered data" })
        );

        // Rebuild on an empty tree returns the (zero-node) status, not an error.
        let rebuilt = tree_summarizer_rebuild(&cfg, "team")
            .await
            .expect("rebuild should not error on the provider precondition when opted in");
        assert_eq!(rebuilt.value["total_nodes"], 0);
    }
}
