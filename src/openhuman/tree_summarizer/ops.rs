//! RPC operation wrappers for the tree summarizer.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::providers;
use crate::openhuman::tree_summarizer::{engine, store, types::*};
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

    let provider = create_provider(config)?;
    let ts = Utc::now();

    match engine::run_summarization(config, provider.as_ref(), namespace.trim(), ts).await {
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

    let provider = create_provider(config)?;

    let status = engine::rebuild_tree(config, provider.as_ref(), namespace.trim())
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

fn create_provider(
    config: &Config,
) -> Result<Box<dyn crate::openhuman::providers::traits::Provider>, String> {
    providers::create_resilient_provider(
        config.api_key.as_deref(),
        config.api_url.as_deref(),
        &config.reliability,
    )
    .map_err(|e| format!("failed to create provider: {e}"))
}
