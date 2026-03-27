//! State and data ops: published state get/set, filesystem data read/write.

use parking_lot::RwLock;
use rquickjs::{Ctx, Function, Object};
use serde::Deserialize;
use std::sync::Arc;
use tauri::Manager;
use tinyhumansai::{Priority, SourceType};

use super::types::{js_err, SkillContext, SkillState};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsMemoryInsertInput {
    title: String,
    content: String,
    source_type: Option<String>,
    metadata: Option<serde_json::Value>,
    priority: Option<String>,
    created_at: Option<f64>,
    updated_at: Option<f64>,
    document_id: Option<String>,
}

pub fn register<'js>(
    ctx: &Ctx<'js>,
    ops: &Object<'js>,
    skill_state: Arc<RwLock<SkillState>>,
    skill_context: SkillContext,
) -> rquickjs::Result<()> {
    // ========================================================================
    // State Bridge (3)
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
    // Data Bridge (2)
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
    // Memory Bridge (1)
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
                    if input.title.trim().is_empty() {
                        return Err(js_err("memory.insert requires a non-empty title"));
                    }
                    if input.content.trim().is_empty() {
                        return Err(js_err("memory.insert requires non-empty content"));
                    }

                    let app_handle = sc
                        .app_handle
                        .clone()
                        .ok_or_else(|| js_err("App handle not available for memory insert"))?;

                    let memory_state = app_handle
                        .try_state::<crate::memory::MemoryState>()
                        .ok_or_else(|| js_err("Memory state not available"))?;

                    let client_opt = memory_state
                        .0
                        .lock()
                        .map_err(|_| js_err("Failed to lock memory state"))?
                        .clone();

                    let client =
                        client_opt.ok_or_else(|| js_err("Memory client is not initialized"))?;
                    let skill_id = sc.skill_id.clone();
                    let integration_id = sc.skill_id.clone();
                    let source_type = match input.source_type.as_deref() {
                        Some("doc") => Some(SourceType::Doc),
                        Some("chat") => Some(SourceType::Chat),
                        Some("email") => Some(SourceType::Email),
                        Some(_) => {
                            return Err(js_err("sourceType must be one of: doc, chat, email"))
                        }
                        None => None,
                    };
                    let priority = match input.priority.as_deref() {
                        Some("high") => Some(Priority::High),
                        Some("medium") => Some(Priority::Medium),
                        Some("low") => Some(Priority::Low),
                        Some(_) => {
                            return Err(js_err("priority must be one of: high, medium, low"))
                        }
                        None => None,
                    };
                    let metadata = input.metadata.unwrap_or_else(|| serde_json::json!({}));

                    tokio::spawn(async move {
                        if let Err(e) = client
                            .store_skill_sync(
                                &skill_id,
                                &integration_id,
                                &input.title,
                                &input.content,
                                source_type,
                                Some(metadata),
                                priority,
                                input.created_at,
                                input.updated_at,
                                input.document_id,
                            )
                            .await
                        {
                            log::warn!(
                                "[quickjs] memory_insert failed for '{}': {}",
                                integration_id,
                                e
                            );
                        } else {
                            log::info!(
                                "[quickjs] memory_insert stored '{}': title='{}'",
                                integration_id,
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
