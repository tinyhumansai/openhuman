//! Persistent history of accepted autocomplete completions.
//!
//! Accepted completions are stored in the local KV store under the
//! "autocomplete" namespace and fed back as dynamic style examples on the
//! next inference cycle, giving the model in-context personalisation.

use crate::openhuman::memory::{InsertMemoryParams, MemoryClient, NamespaceDocumentInput};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tinyhumansai::{Priority, SourceType};

const AUTOCOMPLETE_KV_NAMESPACE: &str = "autocomplete";
const AUTOCOMPLETE_DOC_NAMESPACE: &str = "autocomplete-memory";
const AUTOCOMPLETE_CLOUD_NAMESPACE: &str = "autocomplete";
const MAX_HISTORY_ENTRIES: usize = 50;
const MAX_DOC_ENTRIES: usize = 200;
const CONTEXT_TAIL_CHARS: usize = 40;

/// A single accepted completion record persisted in the KV store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptedCompletion {
    pub context: String,
    pub suggestion: String,
    pub app_name: Option<String>,
    pub timestamp_ms: i64,
}

/// Persist an accepted completion to the local KV store (fire-and-forget safe).
///
/// Keys are zero-padded timestamps so lexicographic order == chronological order.
/// After saving, old entries beyond `MAX_HISTORY_ENTRIES` are trimmed.
pub async fn save_accepted_completion(context: &str, suggestion: &str, app_name: Option<&str>) {
    let client = match MemoryClient::new_local() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[autocomplete:history] client init failed: {e}");
            return;
        }
    };

    let ts_ms = Utc::now().timestamp_millis();
    let key = format!("accepted:{ts_ms:018}");
    let entry = AcceptedCompletion {
        context: context.to_string(),
        suggestion: suggestion.to_string(),
        app_name: app_name.map(str::to_string),
        timestamp_ms: ts_ms,
    };
    let value = match serde_json::to_value(&entry) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[autocomplete:history] serialise failed: {e}");
            return;
        }
    };

    if let Err(e) = client
        .kv_set(Some(AUTOCOMPLETE_KV_NAMESPACE), &key, &value)
        .await
    {
        log::warn!("[autocomplete:history] kv_set failed: {e}");
        return;
    }

    log::debug!("[autocomplete:history] saved accepted completion key={key}");

    // Trim to MAX_HISTORY_ENTRIES — list is returned newest-first.
    if let Ok(rows) = client.kv_list_namespace(AUTOCOMPLETE_KV_NAMESPACE).await {
        if rows.len() > MAX_HISTORY_ENTRIES {
            // rows is newest-first; delete from index MAX_HISTORY_ENTRIES onward (oldest).
            for row in rows.into_iter().skip(MAX_HISTORY_ENTRIES) {
                if let Some(k) = row["key"].as_str() {
                    let _ = client.kv_delete(Some(AUTOCOMPLETE_KV_NAMESPACE), k).await;
                }
            }
        }
    }
}

/// Persist an accepted completion as a local memory document (fire-and-forget safe).
///
/// Documents are stored in the `"autocomplete-memory"` namespace and are
/// searchable via `query_namespace`, enabling semantic matching of past
/// completions against the current typing context.
pub async fn save_completion_to_local_docs(
    context: &str,
    suggestion: &str,
    app_name: Option<&str>,
) {
    let client = match MemoryClient::new_local() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[autocomplete:history] local doc — client init failed: {e}");
            return;
        }
    };

    let ts_ms = Utc::now().timestamp_millis();
    let key = format!("completion:{ts_ms:018}");
    let app = app_name.unwrap_or("unknown");

    // Build the same formatted string used by load_recent_examples so that
    // query results are directly usable as style examples in inference.
    let tail: String = context
        .chars()
        .rev()
        .take(CONTEXT_TAIL_CHARS)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let formatted = format!("[{app}] ...{tail} → {suggestion}");

    let mut tags = vec!["autocomplete".to_string(), "accepted".to_string()];
    if let Some(name) = app_name {
        tags.push(name.to_string());
    }

    let input = NamespaceDocumentInput {
        namespace: AUTOCOMPLETE_DOC_NAMESPACE.to_string(),
        key,
        title: format!("Accepted completion — {app}"),
        content: formatted,
        source_type: "autocomplete".to_string(),
        priority: "low".to_string(),
        tags,
        metadata: json!({
            "context": context,
            "suggestion": suggestion,
            "app_name": app_name,
            "timestamp_ms": ts_ms,
        }),
        category: "daily".to_string(),
        session_id: None,
        document_id: None,
    };

    if let Err(e) = client.put_doc(input).await {
        log::warn!("[autocomplete:history] local doc put_doc failed: {e}");
        return;
    }

    log::debug!("[autocomplete:history] saved local doc completion ts={ts_ms}");

    // Trim to MAX_DOC_ENTRIES — delete oldest documents beyond the limit.
    if let Ok(docs) = client
        .list_documents(Some(AUTOCOMPLETE_DOC_NAMESPACE))
        .await
    {
        let items = docs
            .get("documents")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        if items.len() > MAX_DOC_ENTRIES {
            for item in items.into_iter().skip(MAX_DOC_ENTRIES) {
                if let Some(doc_id) = item.get("documentId").and_then(serde_json::Value::as_str) {
                    let _ = client
                        .delete_document(AUTOCOMPLETE_DOC_NAMESPACE, doc_id)
                        .await;
                }
            }
        }
    }
}

/// Persist an accepted completion to the Neocortex cloud memory graph (fire-and-forget safe).
///
/// No-op when the user is not authenticated (no JWT_TOKEN env var).
/// Neocortex manages its own retention so no client-side trimming is needed.
pub async fn save_completion_to_cloud(context: &str, suggestion: &str, app_name: Option<&str>) {
    let client = match MemoryClient::new_local() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[autocomplete:history] cloud — client init failed: {e}");
            return;
        }
    };

    let ts_ms = Utc::now().timestamp_millis();
    let app = app_name.unwrap_or("unknown");

    let tail: String = context
        .chars()
        .rev()
        .take(CONTEXT_TAIL_CHARS)
        .collect::<String>()
        .chars()
        .rev()
        .collect();

    let params = InsertMemoryParams {
        title: format!("Autocomplete acceptance — {app}"),
        content: format!("Context: ...{tail}\nAccepted: {suggestion}\nApp: {app}"),
        namespace: AUTOCOMPLETE_CLOUD_NAMESPACE.to_string(),
        document_id: format!("ac-{ts_ms}"),
        source_type: Some(SourceType::Doc),
        priority: Some(Priority::Low),
        metadata: Some(json!({
            "context": context,
            "suggestion": suggestion,
            "app_name": app_name,
            "timestamp_ms": ts_ms,
        })),
        created_at: None,
        updated_at: None,
    };

    if let Err(e) = client.insert_to_cloud(params).await {
        log::warn!("[autocomplete:history] cloud insert_memory failed: {e}");
    } else {
        log::debug!("[autocomplete:history] saved cloud completion ts={ts_ms}");
    }
}

/// Query the local document store for accepted completions semantically
/// relevant to the current typing `context`.
///
/// Uses `query_namespace` (keyword + optional vector ranking) against the
/// `"autocomplete-memory"` namespace. Returns up to `n` formatted style
/// example strings ready for injection into the inference prompt.
pub async fn query_relevant_examples(context: &str, n: usize) -> Vec<String> {
    let client = match MemoryClient::new_local() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[autocomplete:history] query_relevant — client init failed: {e}");
            return Vec::new();
        }
    };

    // Use the tail of the current context as the search query.
    let tail: String = context
        .chars()
        .rev()
        .take(80)
        .collect::<String>()
        .chars()
        .rev()
        .collect();

    let result = match client
        .query_namespace(AUTOCOMPLETE_DOC_NAMESPACE, &tail, n as u32)
        .await
    {
        Ok(r) if !r.is_empty() => r,
        Ok(_) => return Vec::new(),
        Err(e) => {
            log::warn!("[autocomplete:history] query_namespace failed: {e}");
            return Vec::new();
        }
    };

    // query_namespace_context returns "key: content" entries joined by "\n\n".
    // The content is already in "[app] ...tail → suggestion" format.
    result
        .split("\n\n")
        .filter(|s| !s.is_empty())
        .filter_map(|entry| {
            // Strip the "completion:XXXXXXXXXXXXXXXXXX: " key prefix.
            let bracket_pos = entry.find('[')?;
            Some(entry[bracket_pos..].to_string())
        })
        .take(n)
        .collect()
}

/// Load the `n` most recent accepted completions as formatted style example strings.
///
/// Each string has the form: `"[AppName] ...{tail} → suggestion"`
/// These are prepended to the user's static style examples before inference.
pub async fn load_recent_examples(n: usize) -> Vec<String> {
    let client = match MemoryClient::new_local() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[autocomplete:history] load examples — client init failed: {e}");
            return Vec::new();
        }
    };

    let rows = match client.kv_list_namespace(AUTOCOMPLETE_KV_NAMESPACE).await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[autocomplete:history] kv_list_namespace failed: {e}");
            return Vec::new();
        }
    };

    rows.into_iter()
        .take(n)
        .filter_map(|row| {
            let val = row.get("value")?;
            let entry: AcceptedCompletion = serde_json::from_value(val.clone()).ok()?;
            let tail: String = entry
                .context
                .chars()
                .rev()
                .take(CONTEXT_TAIL_CHARS)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            let app = entry.app_name.as_deref().unwrap_or("unknown");
            Some(format!("[{app}] ...{tail} → {}", entry.suggestion))
        })
        .collect()
}

/// Return up to `limit` recent accepted completions (newest first), for the settings UI.
pub async fn list_history(limit: usize) -> Result<Vec<AcceptedCompletion>, String> {
    let client = MemoryClient::new_local()?;
    let rows = client.kv_list_namespace(AUTOCOMPLETE_KV_NAMESPACE).await?;
    let entries = rows
        .into_iter()
        .take(limit)
        .filter_map(|row| {
            let val = row.get("value")?;
            serde_json::from_value::<AcceptedCompletion>(val.clone()).ok()
        })
        .collect();
    Ok(entries)
}

/// Delete all accepted-completion entries across all layers.
/// Returns the total number of entries removed (KV + local docs).
pub async fn clear_history() -> Result<usize, String> {
    let client = MemoryClient::new_local()?;

    // 1. Clear KV entries (existing behaviour — powers the UI list).
    let rows = client.kv_list_namespace(AUTOCOMPLETE_KV_NAMESPACE).await?;
    let kv_count = rows.len();
    for row in &rows {
        if let Some(k) = row["key"].as_str() {
            let _ = client.kv_delete(Some(AUTOCOMPLETE_KV_NAMESPACE), k).await;
        }
    }

    // 2. Clear local document entries (semantic search layer).
    let doc_count = match client
        .list_documents(Some(AUTOCOMPLETE_DOC_NAMESPACE))
        .await
    {
        Ok(docs) => {
            let items = docs
                .get("documents")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            let count = items.len();
            for item in items {
                if let Some(doc_id) = item.get("documentId").and_then(serde_json::Value::as_str) {
                    let _ = client
                        .delete_document(AUTOCOMPLETE_DOC_NAMESPACE, doc_id)
                        .await;
                }
            }
            count
        }
        Err(e) => {
            log::warn!("[autocomplete:history] clear docs — list_documents failed: {e}");
            0
        }
    };

    // 3. Clear cloud Neocortex entries (best-effort, no-op if unauthenticated).
    if let Err(e) = client
        .delete_cloud_namespace(AUTOCOMPLETE_CLOUD_NAMESPACE)
        .await
    {
        log::warn!("[autocomplete:history] clear cloud namespace failed: {e}");
    }

    let total = kv_count + doc_count;
    log::debug!(
        "[autocomplete:history] cleared {kv_count} KV + {doc_count} doc entries ({total} total)"
    );
    Ok(total)
}
