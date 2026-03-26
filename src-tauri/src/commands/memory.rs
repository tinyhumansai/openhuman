//! Tauri commands for the TinyHumans memory layer.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use crate::memory::{MemoryClient, MemoryClientRef};

/// App-state slot for the memory client.
/// Starts as `None`; populated by `init_memory_client` when the frontend
/// provides the user's JWT token from `authSlice.token`.
pub struct MemoryState(pub Mutex<Option<MemoryClientRef>>);

fn extract_namespaces_from_documents(payload: &serde_json::Value) -> Vec<String> {
    fn collect_from_value(value: &serde_json::Value, out: &mut BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(ns) = map.get("namespace").and_then(serde_json::Value::as_str) {
                    if !ns.trim().is_empty() {
                        out.insert(ns.to_string());
                    }
                }
                for nested in map.values() {
                    collect_from_value(nested, out);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    collect_from_value(item, out);
                }
            }
            _ => {}
        }
    }

    let mut namespaces = BTreeSet::new();
    collect_from_value(payload, &mut namespaces);
    namespaces.into_iter().collect()
}

fn filter_documents_payload_by_namespace(
    payload: serde_json::Value,
    namespace: &str,
) -> serde_json::Value {
    fn filter_array(items: &mut Vec<serde_json::Value>, namespace: &str) {
        items.retain(|item| {
            item.as_object()
                .and_then(|obj| obj.get("namespace"))
                .and_then(serde_json::Value::as_str)
                .map(|ns| ns == namespace)
                .unwrap_or(false)
        });
    }

    match payload {
        serde_json::Value::Array(mut items) => {
            filter_array(&mut items, namespace);
            serde_json::Value::Array(items)
        }
        serde_json::Value::Object(mut root) => {
            for key in ["documents", "items", "results"] {
                if let Some(serde_json::Value::Array(items)) = root.get_mut(key) {
                    filter_array(items, namespace);
                    return serde_json::Value::Object(root);
                }
            }

            if let Some(serde_json::Value::Object(data)) = root.get_mut("data") {
                for key in ["documents", "items", "results"] {
                    if let Some(serde_json::Value::Array(items)) = data.get_mut(key) {
                        filter_array(items, namespace);
                        return serde_json::Value::Object(root);
                    }
                }
            }

            serde_json::Value::Object(root)
        }
        other => other,
    }
}

/// Called by the frontend with the JWT from `authSlice.token`.
/// (Re-)initialises the TinyHumans memory client for the current session.
#[tauri::command]
pub async fn init_memory_client(
    jwt_token: String,
    state: tauri::State<'_, MemoryState>,
) -> Result<(), String> {
    log::info!("[memory] init_memory_client: entry (token_present={})", !jwt_token.trim().is_empty());
    let client = MemoryClient::from_token(jwt_token).map(Arc::new);
    if client.is_none() {
        log::warn!("[memory] init_memory_client: exit — empty token, memory layer disabled");
    } else {
        log::info!("[memory] init_memory_client: exit — client ready");
    }
    *state.0.lock().map_err(|e| e.to_string())? = client;
    Ok(())
}

/// Recall context from the TinyHumans Master memory node for a skill integration.
/// Returns the recalled context string (or null if the server had nothing to return).
#[tauri::command]
pub async fn recall_memory(
    skill_id: String,
    integration_id: String,
    max_chunks: Option<u32>,
    state: tauri::State<'_, MemoryState>,
) -> Result<Option<String>, String> {
    log::info!(
        "[memory] recall_memory: entry (skill_id={skill_id}, integration_id={integration_id}, max_chunks={max_chunks:?})"
    );
    let client = state.0.lock().map_err(|e| e.to_string())?.clone();
    match client {
        Some(c) => {
            let result = c
                .recall_skill_context(&skill_id, &integration_id, max_chunks.unwrap_or(10))
                .await;
            match &result {
                Ok(ctx) => log::info!(
                    "[memory] recall_memory: exit — ok (has_context={})",
                    ctx.is_some()
                ),
                Err(e) => log::warn!("[memory] recall_memory: exit — error: {e}"),
            }
            result.map(|ctx| ctx.map(|ctx| ctx.to_string()))
        }
        None => {
            log::warn!("[memory] recall_memory: exit — client not initialised (no JWT set)");
            Err("Memory layer not configured — JWT token not yet set".into())
        }
    }
}

/// Query the TinyHumans memory for a skill integration.
/// Returns the RAG context string to inject into AI prompts.
#[tauri::command]
pub async fn memory_query(
    skill_id: String,
    integration_id: String,
    query: String,
    max_chunks: Option<u32>,
    state: tauri::State<'_, MemoryState>,
) -> Result<String, String> {
    log::info!("[memory] memory_query: entry (skill_id={skill_id}, integration_id={integration_id}, max_chunks={max_chunks:?})");
    let client = state.0.lock().map_err(|e| e.to_string())?.clone();
    match client {
        Some(c) => {
            let result = c
                .query_skill_context(&skill_id, &integration_id, &query, max_chunks.unwrap_or(10))
                .await;
            match &result {
                Ok(ctx) => log::info!("[memory] memory_query: exit — ok (context_len={})", ctx.len()),
                Err(e) => log::warn!("[memory] memory_query: exit — error: {e}"),
            }
            result
        }
        None => {
            log::warn!("[memory] memory_query: exit — client not initialised (no JWT set)");
            Err("Memory layer not configured — JWT token not yet set".into())
        }
    }
}

#[tauri::command]
pub async fn memory_list_documents(
    namespace: Option<String>,
    state: tauri::State<'_, MemoryState>,
) -> Result<serde_json::Value, String> {
    let client = state.0.lock().map_err(|e| e.to_string())?.clone();
    match client {
        Some(c) => {
            let docs = c.list_documents().await?;
            let filtered = namespace
                .as_deref()
                .map(str::trim)
                .filter(|ns| !ns.is_empty())
                .map(|ns| filter_documents_payload_by_namespace(docs.clone(), ns))
                .unwrap_or(docs);
            Ok(filtered)
        }
        None => Err("Memory layer not configured — JWT token not yet set".into()),
    }
}

#[tauri::command]
pub async fn memory_list_namespaces(
    state: tauri::State<'_, MemoryState>,
) -> Result<Vec<String>, String> {
    let client = state.0.lock().map_err(|e| e.to_string())?.clone();
    match client {
        Some(c) => {
            let docs = c.list_documents().await?;
            Ok(extract_namespaces_from_documents(&docs))
        }
        None => Err("Memory layer not configured — JWT token not yet set".into()),
    }
}

#[tauri::command]
pub async fn memory_delete_document(
    document_id: String,
    namespace: String,
    state: tauri::State<'_, MemoryState>,
) -> Result<serde_json::Value, String> {
    let client = state.0.lock().map_err(|e| e.to_string())?.clone();
    match client {
        Some(c) => c.delete_document(&document_id, &namespace).await,
        None => Err("Memory layer not configured — JWT token not yet set".into()),
    }
}

#[tauri::command]
pub async fn memory_query_namespace(
    namespace: String,
    query: String,
    max_chunks: Option<u32>,
    state: tauri::State<'_, MemoryState>,
) -> Result<String, String> {
    let client = state.0.lock().map_err(|e| e.to_string())?.clone();
    match client {
        Some(c) => c
            .query_namespace_context(&namespace, &query, max_chunks.unwrap_or(10))
            .await,
        None => Err("Memory layer not configured — JWT token not yet set".into()),
    }
}

#[tauri::command]
pub async fn memory_recall_namespace(
    namespace: String,
    max_chunks: Option<u32>,
    state: tauri::State<'_, MemoryState>,
) -> Result<Option<String>, String> {
    let client = state.0.lock().map_err(|e| e.to_string())?.clone();
    match client {
        Some(c) => c
            .recall_namespace_context(&namespace, max_chunks.unwrap_or(10))
            .await,
        None => Err("Memory layer not configured — JWT token not yet set".into()),
    }
}
