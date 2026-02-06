//! Tauri commands for the V8 skill runtime.
//!
//! These commands expose the RuntimeEngine to the frontend WebView
//! and serve as the bridge between the ephemeral UI and the persistent runtime.
//!
//! Note: V8 runtime is only available on desktop platforms.
//! On mobile, these commands return appropriate errors or empty results.

use crate::models::socket::SocketState;
use crate::runtime::socket_manager::SocketManager;
use crate::utils::config::get_backend_url;
use std::sync::Arc;
use tauri::State;

// Desktop-only imports
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::runtime::qjs_engine::RuntimeEngine;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::runtime::types::{SkillSnapshot, ToolResult};

// Mobile stub types
#[cfg(any(target_os = "android", target_os = "ios"))]
use serde::{Deserialize, Serialize};

#[cfg(any(target_os = "android", target_os = "ios"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSnapshot {
    pub id: String,
    pub name: String,
    pub state: String,
    pub tools: Vec<serde_json::Value>,
    pub error: Option<String>,
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: Vec<serde_json::Value>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

// =============================================================================
// Desktop implementations (V8 available)
// =============================================================================

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod desktop {
    use super::*;

    /// List all skills discovered from the skills directory (including not-yet-started).
    #[tauri::command]
    pub async fn runtime_discover_skills(
        engine: State<'_, Arc<RuntimeEngine>>,
    ) -> Result<Vec<serde_json::Value>, String> {
        let manifests = engine.discover_skills().await?;
        let result: Vec<serde_json::Value> = manifests
            .iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "name": m.name,
                    "runtime": m.runtime,
                    "entry": m.entry,
                    "autoStart": m.auto_start,
                    "version": m.version,
                    "description": m.description,
                    "setup": m.setup.as_ref().map(|s| {
                        let mut obj = serde_json::json!({
                            "required": s.required,
                            "label": s.label,
                        });
                        if let Some(oauth) = &s.oauth {
                            obj["oauth"] = oauth.clone();
                        }
                        obj
                    }),
                    "platforms": m.platforms,
                })
            })
            .collect();
        Ok(result)
    }

    /// List all currently registered (running/stopped/error) skill instances.
    #[tauri::command]
    pub async fn runtime_list_skills(
        engine: State<'_, Arc<RuntimeEngine>>,
    ) -> Result<Vec<SkillSnapshot>, String> {
        Ok(engine.list_skills())
    }

    /// Start a skill by its ID.
    #[tauri::command]
    pub async fn runtime_start_skill(
        engine: State<'_, Arc<RuntimeEngine>>,
        skill_id: String,
    ) -> Result<SkillSnapshot, String> {
        engine.start_skill(&skill_id).await
    }

    /// Stop a running skill.
    #[tauri::command]
    pub async fn runtime_stop_skill(
        engine: State<'_, Arc<RuntimeEngine>>,
        skill_id: String,
    ) -> Result<(), String> {
        engine.stop_skill(&skill_id).await
    }

    /// Get the current state of a specific skill.
    #[tauri::command]
    pub async fn runtime_get_skill_state(
        engine: State<'_, Arc<RuntimeEngine>>,
        skill_id: String,
    ) -> Result<Option<SkillSnapshot>, String> {
        Ok(engine.get_skill_state(&skill_id))
    }

    /// Call a tool on a specific skill.
    #[tauri::command]
    pub async fn runtime_call_tool(
        engine: State<'_, Arc<RuntimeEngine>>,
        skill_id: String,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<ToolResult, String> {
        engine.call_tool(&skill_id, &tool_name, arguments).await
    }

    /// List all tool definitions across all running skills.
    #[tauri::command]
    pub async fn runtime_all_tools(
        engine: State<'_, Arc<RuntimeEngine>>,
    ) -> Result<Vec<serde_json::Value>, String> {
        let tools = engine.all_tools();
        Ok(tools
            .into_iter()
            .map(|(skill_id, tool)| {
                serde_json::json!({
                    "skillId": skill_id,
                    "tool": tool,
                })
            })
            .collect())
    }

    /// Broadcast an event to all running skills.
    #[tauri::command]
    pub async fn runtime_broadcast_event(
        engine: State<'_, Arc<RuntimeEngine>>,
        event: String,
        data: serde_json::Value,
    ) -> Result<(), String> {
        engine.broadcast_event(&event, data).await;
        Ok(())
    }

    /// Enable a skill: persist preference and start it.
    #[tauri::command]
    pub async fn runtime_enable_skill(
        engine: State<'_, Arc<RuntimeEngine>>,
        skill_id: String,
    ) -> Result<(), String> {
        engine.enable_skill(&skill_id).await
    }

    /// Disable a skill: persist preference and stop it.
    #[tauri::command]
    pub async fn runtime_disable_skill(
        engine: State<'_, Arc<RuntimeEngine>>,
        skill_id: String,
    ) -> Result<(), String> {
        engine.disable_skill(&skill_id).await
    }

    /// Check whether a skill is enabled (preference or manifest default).
    #[tauri::command]
    pub async fn runtime_is_skill_enabled(
        engine: State<'_, Arc<RuntimeEngine>>,
        skill_id: String,
    ) -> Result<bool, String> {
        Ok(engine.is_skill_enabled(&skill_id))
    }

    /// Get all stored skill preferences.
    #[tauri::command]
    pub async fn runtime_get_skill_preferences(
        engine: State<'_, Arc<RuntimeEngine>>,
    ) -> Result<serde_json::Value, String> {
        let prefs = engine.get_preferences();
        serde_json::to_value(prefs).map_err(|e| format!("Failed to serialize preferences: {e}"))
    }

    /// Read a KV value from a skill's database.
    #[tauri::command]
    pub async fn runtime_skill_kv_get(
        engine: State<'_, Arc<RuntimeEngine>>,
        skill_id: String,
        key: String,
    ) -> Result<serde_json::Value, String> {
        engine.kv_get(&skill_id, &key)
    }

    /// Write a KV value into a skill's database.
    #[tauri::command]
    pub async fn runtime_skill_kv_set(
        engine: State<'_, Arc<RuntimeEngine>>,
        skill_id: String,
        key: String,
        value: serde_json::Value,
    ) -> Result<(), String> {
        engine.kv_set(&skill_id, &key, &value)
    }

    /// Route a JSON-RPC method call to the V8 skill engine.
    #[tauri::command]
    pub async fn runtime_rpc(
        engine: State<'_, Arc<RuntimeEngine>>,
        skill_id: String,
        method: String,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        engine.rpc(&skill_id, &method, params).await
    }

    /// Read a file from a skill's data directory.
    #[tauri::command]
    pub async fn runtime_skill_data_read(
        engine: State<'_, Arc<RuntimeEngine>>,
        skill_id: String,
        filename: String,
    ) -> Result<String, String> {
        engine.data_read(&skill_id, &filename)
    }

    /// Write a file to a skill's data directory.
    #[tauri::command]
    pub async fn runtime_skill_data_write(
        engine: State<'_, Arc<RuntimeEngine>>,
        skill_id: String,
        filename: String,
        content: String,
    ) -> Result<(), String> {
        engine.data_write(&skill_id, &filename, &content)
    }

    /// Get the data directory path for a skill.
    #[tauri::command]
    pub async fn runtime_skill_data_dir(
        engine: State<'_, Arc<RuntimeEngine>>,
        skill_id: String,
    ) -> Result<String, String> {
        Ok(engine
            .skill_data_dir(&skill_id)
            .to_string_lossy()
            .to_string())
    }
}

// =============================================================================
// Mobile stub implementations (V8 not available)
// =============================================================================

#[cfg(any(target_os = "android", target_os = "ios"))]
mod mobile {
    use super::*;

    const MOBILE_ERROR: &str = "V8 skill runtime is not available on mobile platforms";

    #[tauri::command]
    pub async fn runtime_discover_skills() -> Result<Vec<serde_json::Value>, String> {
        // Return empty list on mobile
        Ok(vec![])
    }

    #[tauri::command]
    pub async fn runtime_list_skills() -> Result<Vec<SkillSnapshot>, String> {
        Ok(vec![])
    }

    #[tauri::command]
    pub async fn runtime_start_skill(_skill_id: String) -> Result<SkillSnapshot, String> {
        Err(MOBILE_ERROR.to_string())
    }

    #[tauri::command]
    pub async fn runtime_stop_skill(_skill_id: String) -> Result<(), String> {
        Err(MOBILE_ERROR.to_string())
    }

    #[tauri::command]
    pub async fn runtime_get_skill_state(_skill_id: String) -> Result<Option<SkillSnapshot>, String> {
        Ok(None)
    }

    #[tauri::command]
    pub async fn runtime_call_tool(
        _skill_id: String,
        _tool_name: String,
        _arguments: serde_json::Value,
    ) -> Result<ToolResult, String> {
        Err(MOBILE_ERROR.to_string())
    }

    #[tauri::command]
    pub async fn runtime_all_tools() -> Result<Vec<serde_json::Value>, String> {
        Ok(vec![])
    }

    #[tauri::command]
    pub async fn runtime_broadcast_event(
        _event: String,
        _data: serde_json::Value,
    ) -> Result<(), String> {
        // Silent no-op on mobile
        Ok(())
    }

    #[tauri::command]
    pub async fn runtime_enable_skill(_skill_id: String) -> Result<(), String> {
        Err(MOBILE_ERROR.to_string())
    }

    #[tauri::command]
    pub async fn runtime_disable_skill(_skill_id: String) -> Result<(), String> {
        Err(MOBILE_ERROR.to_string())
    }

    #[tauri::command]
    pub async fn runtime_is_skill_enabled(_skill_id: String) -> Result<bool, String> {
        Ok(false)
    }

    #[tauri::command]
    pub async fn runtime_get_skill_preferences() -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({}))
    }

    #[tauri::command]
    pub async fn runtime_skill_kv_get(_skill_id: String, _key: String) -> Result<serde_json::Value, String> {
        Err(MOBILE_ERROR.to_string())
    }

    #[tauri::command]
    pub async fn runtime_skill_kv_set(
        _skill_id: String,
        _key: String,
        _value: serde_json::Value,
    ) -> Result<(), String> {
        Err(MOBILE_ERROR.to_string())
    }

    #[tauri::command]
    pub async fn runtime_rpc(
        _skill_id: String,
        _method: String,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err(MOBILE_ERROR.to_string())
    }

    #[tauri::command]
    pub async fn runtime_skill_data_read(_skill_id: String, _filename: String) -> Result<String, String> {
        Err(MOBILE_ERROR.to_string())
    }

    #[tauri::command]
    pub async fn runtime_skill_data_write(
        _skill_id: String,
        _filename: String,
        _content: String,
    ) -> Result<(), String> {
        Err(MOBILE_ERROR.to_string())
    }

    #[tauri::command]
    pub async fn runtime_skill_data_dir(_skill_id: String) -> Result<String, String> {
        Err(MOBILE_ERROR.to_string())
    }
}

// =============================================================================
// Socket.io commands (available on all platforms)
// =============================================================================

/// Connect the Rust-native Socket.io client to the backend.
#[tauri::command]
pub async fn runtime_socket_connect(
    socket_mgr: State<'_, Arc<SocketManager>>,
    token: String,
    url: Option<String>,
) -> Result<(), String> {
    let backend_url = url.unwrap_or_else(get_backend_url);
    log::info!("[socket-cmd] runtime_socket_connect to {}", backend_url);
    socket_mgr.connect(&backend_url, &token).await
}

/// Disconnect the Rust-native Socket.io client.
#[tauri::command]
pub async fn runtime_socket_disconnect(
    socket_mgr: State<'_, Arc<SocketManager>>,
) -> Result<(), String> {
    socket_mgr.disconnect().await
}

/// Get the current Rust socket connection state.
#[tauri::command]
pub async fn runtime_socket_state(
    socket_mgr: State<'_, Arc<SocketManager>>,
) -> Result<SocketState, String> {
    Ok(socket_mgr.get_state())
}

/// Emit an event through the Rust socket to the server.
#[tauri::command]
pub async fn runtime_socket_emit(
    socket_mgr: State<'_, Arc<SocketManager>>,
    event: String,
    data: serde_json::Value,
) -> Result<(), String> {
    socket_mgr.emit(&event, data).await
}

// =============================================================================
// Re-exports based on platform
// =============================================================================

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use desktop::*;

#[cfg(any(target_os = "android", target_os = "ios"))]
pub use mobile::*;
