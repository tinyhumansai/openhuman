//! State and memory operations: skill state get/set, filesystem data access, and memory insertion.

use crate::openhuman::memory::NamespaceDocumentInput;
use parking_lot::RwLock;
use rquickjs::{Ctx, Function, Object};
use serde::Deserialize;
use std::sync::Arc;

use super::types::{js_err, SkillContext, SkillState};

/// Input structure for the `memory_insert` operation.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsMemoryInsertInput {
    /// Title of the document to be stored in memory.
    title: String,
    /// Content of the document.
    content: String,
    /// Type of the source (e.g., "doc", "chat", "email").
    source_type: Option<String>,
    /// Optional metadata associated with the document.
    metadata: Option<serde_json::Value>,
    /// Priority of the document ("high", "medium", "low").
    priority: Option<String>,
    /// Unix timestamp when the document was created.
    created_at: Option<f64>,
    /// Unix timestamp when the document was last updated.
    updated_at: Option<f64>,
    /// Optional unique identifier for the document.
    document_id: Option<String>,
}

/// Registers state and memory operations onto the provided JavaScript object.
pub fn register<'js>(
    ctx: &Ctx<'js>,
    ops: &Object<'js>,
    skill_state: Arc<RwLock<SkillState>>,
    skill_context: SkillContext,
) -> rquickjs::Result<()> {
    // ========================================================================
    // State Bridge
    // Allows skills to get and set shared state.
    // ========================================================================

    {
        let ss = skill_state.clone();
        ops.set(
            "state_get",
            Function::new(
                ctx.clone(),
                move |key: String| -> rquickjs::Result<String> {
                    let state = ss.read();
                    let value = state
                        .data
                        .get(&key)
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    serde_json::to_string(&value).map_err(|e| js_err(e.to_string()))
                },
            ),
        )?;
    }

    {
        let ss = skill_state.clone();
        ops.set(
            "state_set",
            Function::new(
                ctx.clone(),
                move |key: String, value_json: String| -> rquickjs::Result<()> {
                    let value: serde_json::Value =
                        serde_json::from_str(&value_json).map_err(|e| js_err(e.to_string()))?;
                    let mut state = ss.write();
                    state.data.insert(key, value);
                    state.dirty = true;
                    Ok(())
                },
            ),
        )?;
    }

    {
        let ss = skill_state;
        ops.set(
            "state_set_partial",
            Function::new(
                ctx.clone(),
                move |partial_json: String| -> rquickjs::Result<()> {
                    let partial: serde_json::Map<String, serde_json::Value> =
                        serde_json::from_str(&partial_json).map_err(|e| js_err(e.to_string()))?;
                    let mut state = ss.write();
                    for (k, v) in partial {
                        state.data.insert(k, v);
                    }
                    state.dirty = true;
                    Ok(())
                },
            ),
        )?;
    }

    // ========================================================================
    // Data Bridge
    // Direct filesystem access within the skill's data directory.
    // ========================================================================

    {
        let sc = skill_context.clone();
        ops.set(
            "data_read",
            Function::new(
                ctx.clone(),
                move |filename: String| -> rquickjs::Result<String> {
                    let path = sc.data_dir.join(&filename);
                    std::fs::read_to_string(&path).map_err(|e| js_err(e.to_string()))
                },
            ),
        )?;
    }

    {
        let sc = skill_context.clone();
        ops.set(
            "data_write",
            Function::new(
                ctx.clone(),
                move |filename: String, content: String| -> rquickjs::Result<()> {
                    let path = sc.data_dir.join(&filename);
                    std::fs::write(&path, content).map_err(|e| js_err(e.to_string()))
                },
            ),
        )?;
    }

    // ========================================================================
    // Memory Bridge
    // Integration with the OpenHuman memory system.
    // ========================================================================

    {
        let sc = skill_context;
        ops.set(
            "memory_insert",
            Function::new(
                ctx.clone(),
                move |metadata_json: String| -> rquickjs::Result<()> {
                    let input: JsMemoryInsertInput =
                        serde_json::from_str(&metadata_json).map_err(|e| js_err(e.to_string()))?;
                    
                    // Basic validation
                    if input.title.trim().is_empty() {
                        return Err(js_err("memory.insert requires a non-empty title"));
                    }
                    if input.content.trim().is_empty() {
                        return Err(js_err("memory.insert requires non-empty content"));
                    }

                    let client = sc
                        .memory_client
                        .clone()
                        .ok_or_else(|| js_err("Memory client is not initialized"))?;
                    
                    let skill_id = sc.skill_id.clone();
                    let namespace = format!("skill-{skill_id}");
                    
                    let source_type = input
                        .source_type
                        .unwrap_or_else(|| "doc".to_string())
                        .to_ascii_lowercase();
                    if !matches!(source_type.as_str(), "doc" | "chat" | "email") {
                        return Err(js_err("sourceType must be one of: doc, chat, email"));
                    }
                    
                    let priority = input
                        .priority
                        .unwrap_or_else(|| "medium".to_string())
                        .to_ascii_lowercase();
                    if !matches!(priority.as_str(), "high" | "medium" | "low") {
                        return Err(js_err("priority must be one of: high, medium, low"));
                    }
                    
                    let metadata = input.metadata.unwrap_or_else(|| serde_json::json!({}));

                    // Spawn the memory insertion in the background to avoid blocking the JS loop
                    tokio::spawn(async move {
                        if let Err(e) = client
                            .put_doc(NamespaceDocumentInput {
                                namespace,
                                key: input.title.clone(),
                                title: input.title.clone(),
                                content: input.content.clone(),
                                source_type,
                                priority,
                                tags: Vec::new(),
                                metadata,
                                category: "core".to_string(),
                                session_id: None,
                                document_id: input.document_id,
                            })
                            .await
                        {
                            log::warn!("[quickjs] memory_insert failed for '{}': {}", skill_id, e);
                        } else {
                            log::info!(
                                "[quickjs] memory_insert stored '{}': title='{}'",
                                skill_id,
                                input.title
                            );
                        }
                    });

                    Ok(())
                },
            ),
        )?;
    }

    Ok(())
}
