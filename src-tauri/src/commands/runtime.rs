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
use std::collections::HashMap;
use tauri::State;
use serde::{Deserialize, Serialize};

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
// ZeroClaw Format Compatibility Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZeroClawToolSchema {
    #[serde(rename = "type")]
    pub type_field: String,
    pub function: ZeroClawFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZeroClawFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZeroClawToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
    pub execution_time: Option<u64>,
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
                    "ignoreInProduction": m.ignoreInProduction,
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

    // =============================================================================
    // ZeroClaw Format Compatibility Commands
    // =============================================================================

    /// Generate ZeroClaw-compatible tool schemas from all available QuickJS tools.
    /// This bridges the gap between QuickJS runtime and OpenAI function calling format.
    #[tauri::command]
    pub async fn runtime_get_tool_schemas(
        engine: State<'_, Arc<RuntimeEngine>>,
    ) -> Result<Vec<ZeroClawToolSchema>, String> {
        log::info!("🔧 [RUNTIME] Generating ZeroClaw-compatible tool schemas");

        let tools = engine.all_tools();
        log::info!("🔧 [RUNTIME] Found {} tools from engine", tools.len());

        let mut schemas = Vec::new();

        for (skill_id, tool) in tools {
            // Extract tool information from ToolDefinition struct
            let description = if tool.description.is_empty() {
                "No description available".to_string()
            } else {
                tool.description.clone()
            };

            let tool_name = format!("{}_{}", skill_id, tool.name);
            log::info!("🔧 [RUNTIME] Processing tool: {}", tool_name);

            // Convert input schema to OpenAI-compatible format
            let openai_parameters = convert_to_openai_schema(tool.input_schema)?;

            let schema = ZeroClawToolSchema {
                type_field: "function".to_string(),
                function: ZeroClawFunction {
                    name: tool_name,
                    description,
                    parameters: openai_parameters,
                },
            };

            schemas.push(schema);
        }

        log::info!("🔧 [RUNTIME] Generated {} ZeroClaw tool schemas", schemas.len());

        // Log tools that contain 'notion' or 'gmail' for debugging
        let gmail_notion_tools: Vec<String> = schemas.iter()
            .map(|s| &s.function.name)
            .filter(|name| name.to_lowercase().contains("gmail") || name.to_lowercase().contains("notion"))
            .cloned()
            .collect();
        log::info!("🔧 [RUNTIME] Gmail/Notion tools found: {:?}", gmail_notion_tools);
        Ok(schemas)
    }

    /// Execute a specific tool based on agent decision with enhanced validation.
    /// This wraps the existing runtime_call_tool with ZeroClaw format compatibility.
    #[tauri::command]
    pub async fn runtime_execute_tool(
        engine: State<'_, Arc<RuntimeEngine>>,
        tool_id: String,
        args: serde_json::Value,
    ) -> Result<ZeroClawToolResult, String> {
        let start_time = std::time::Instant::now();

        log::info!("🔧 [RUNTIME] Executing ZeroClaw tool: {} with args: {}", tool_id, args);

        // Parse tool_id to get skill_id and tool_name (format: "skill_id_tool_name")
        let (skill_id, tool_name) = match parse_tool_id(&tool_id) {
            Ok((skill, tool)) => {
                log::info!("🔧 [RUNTIME] Parsed tool_id: skill_id='{}', tool_name='{}'", skill, tool);
                (skill, tool)
            }
            Err(e) => {
                log::error!("🔧 [RUNTIME] Failed to parse tool_id '{}': {}", tool_id, e);
                let execution_time = start_time.elapsed().as_millis() as u64;
                return Ok(ZeroClawToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Invalid tool ID format: {}", e)),
                    execution_time: Some(execution_time),
                });
            }
        };

        // Log runtime state before execution
        log::info!("🔧 [RUNTIME] Attempting to call tool '{}' on skill '{}'", tool_name, skill_id);

        // Get available skills for debugging
        let skills = engine.list_skills();
        log::info!("🔧 [RUNTIME] Available skills: {:?}", skills.iter().map(|s| &s.skill_id).collect::<Vec<_>>());

        // Check if the specific skill exists
        if let Some(skill) = skills.iter().find(|s| s.skill_id == skill_id) {
            log::info!("🔧 [RUNTIME] Found skill '{}' with state: {:?}, tools: {:?}",
                      skill_id, skill.state, skill.tools.iter().map(|t| &t.name).collect::<Vec<_>>());
        } else {
            log::error!("🔧 [RUNTIME] Skill '{}' not found in runtime!", skill_id);
        }

        // Execute the tool using the existing command
        log::info!("🔧 [RUNTIME] Calling engine.call_tool('{}', '{}', {})", skill_id, tool_name, args);

        match engine.call_tool(&skill_id, &tool_name, args).await {
            Ok(result) => {
                let execution_time = start_time.elapsed().as_millis() as u64;
                log::info!("🔧 [RUNTIME] Tool execution completed in {}ms, is_error: {}", execution_time, result.is_error);

                if result.is_error {
                    let error_message = result.content
                        .iter()
                        .filter(|c| matches!(c, crate::runtime::types::ToolContent::Text { .. }))
                        .map(|c| match c {
                            crate::runtime::types::ToolContent::Text { text } => text.as_str(),
                            _ => "",
                        })
                        .collect::<Vec<_>>()
                        .join("\n");

                    log::error!("🔧 [RUNTIME] Tool execution failed with error: {}", error_message);

                    Ok(ZeroClawToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(error_message),
                        execution_time: Some(execution_time),
                    })
                } else {
                    let output = result.content
                        .iter()
                        .map(|c| match c {
                            crate::runtime::types::ToolContent::Text { text } => text.clone(),
                            crate::runtime::types::ToolContent::Json { data } => {
                                serde_json::to_string(data).unwrap_or_else(|_| "Invalid JSON".to_string())
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");

                    log::info!("ZeroClaw tool execution completed in {}ms", execution_time);

                    Ok(ZeroClawToolResult {
                        success: true,
                        output,
                        error: None,
                        execution_time: Some(execution_time),
                    })
                }
            }
            Err(e) => {
                let execution_time = start_time.elapsed().as_millis() as u64;
                log::error!("🔧 [RUNTIME] Engine call_tool failed: {}", e);

                Ok(ZeroClawToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(e),
                    execution_time: Some(execution_time),
                })
            }
        }
    }

    // Helper function to parse tool_id format: "skill_id_tool_name"
    pub fn parse_tool_id(tool_id: &str) -> Result<(String, String), String> {
        // Find the first underscore to separate skill_id from tool_name
        if let Some(underscore_pos) = tool_id.find('_') {
            let skill_id = tool_id[..underscore_pos].to_string();
            let tool_name = tool_id[underscore_pos + 1..].to_string();

            if skill_id.is_empty() || tool_name.is_empty() {
                return Err("Tool ID must be in format 'skill_id_tool_name'".to_string());
            }

            Ok((skill_id, tool_name))
        } else {
            Err("Tool ID must contain an underscore separator".to_string())
        }
    }

    // Helper function to convert MCP schema to OpenAI function calling format
    pub fn convert_to_openai_schema(mcp_schema: serde_json::Value) -> Result<serde_json::Value, String> {
        // If it's already in OpenAI format, return as-is
        if mcp_schema.is_object() && mcp_schema.get("type").is_some() {
            return Ok(mcp_schema);
        }

        // Convert basic MCP schema to OpenAI format
        Ok(serde_json::json!({
            "type": "object",
            "properties": mcp_schema.get("properties").cloned().unwrap_or_else(|| serde_json::json!({})),
            "required": mcp_schema.get("required").cloned().unwrap_or_else(|| serde_json::json!([]))
        }))
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

    #[tauri::command]
    pub async fn runtime_get_tool_schemas() -> Result<Vec<ZeroClawToolSchema>, String> {
        Ok(vec![])
    }

    #[tauri::command]
    pub async fn runtime_execute_tool(
        _tool_id: String,
        _args: serde_json::Value,
    ) -> Result<ZeroClawToolResult, String> {
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

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    mod desktop_tests {
        use super::*;
        use crate::runtime::qjs_engine::RuntimeEngine;

        #[tokio::test]
        async fn test_runtime_get_tool_schemas_format() {
            // Note: This test requires a properly initialized RuntimeEngine
            // In a real test environment, you would mock the engine or use a test instance

            // For now, we'll test the struct format and serialization
            let schema = ZeroClawToolSchema {
                type_field: "function".to_string(),
                function: ZeroClawFunction {
                    name: "test_tool".to_string(),
                    description: "A test tool".to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "message": {
                                "type": "string",
                                "description": "Test message"
                            }
                        },
                        "required": ["message"]
                    })
                }
            };

            // Test serialization
            let json = serde_json::to_string(&schema).expect("Should serialize to JSON");
            assert!(json.contains("function"));
            assert!(json.contains("test_tool"));
            assert!(json.contains("A test tool"));

            // Test deserialization
            let deserialized: ZeroClawToolSchema = serde_json::from_str(&json)
                .expect("Should deserialize from JSON");
            assert_eq!(deserialized.type_field, "function");
            assert_eq!(deserialized.function.name, "test_tool");
        }

        #[tokio::test]
        async fn test_zeroclaw_tool_result_format() {
            let result = ZeroClawToolResult {
                success: true,
                output: "Test output".to_string(),
                error: None,
                execution_time: Some(1500)
            };

            // Test serialization
            let json = serde_json::to_string(&result).expect("Should serialize to JSON");
            assert!(json.contains("true"));
            assert!(json.contains("Test output"));
            assert!(json.contains("1500"));

            // Test error case
            let error_result = ZeroClawToolResult {
                success: false,
                output: String::new(),
                error: Some("Tool not found".to_string()),
                execution_time: Some(100)
            };

            let error_json = serde_json::to_string(&error_result).expect("Should serialize error");
            assert!(json.contains("false") || error_json.contains("false"));
            assert!(error_json.contains("Tool not found"));
        }

        #[test]
        fn test_parse_tool_id_valid_formats() {
            // Test valid tool ID formats
            let (skill_id, tool_name) = desktop::parse_tool_id("github_list_issues")
                .expect("Should parse valid tool ID");
            assert_eq!(skill_id, "github");
            assert_eq!(tool_name, "list_issues");

            let (skill_id, tool_name) = desktop::parse_tool_id("notion_create_page")
                .expect("Should parse valid tool ID");
            assert_eq!(skill_id, "notion");
            assert_eq!(tool_name, "create_page");

            // Test complex skill names (first underscore separates skill_id from tool_name)
            let (skill_id, tool_name) = desktop::parse_tool_id("complex_skill_name_tool_function")
                .expect("Should parse complex tool ID");
            assert_eq!(skill_id, "complex");
            assert_eq!(tool_name, "skill_name_tool_function");
        }

        #[test]
        fn test_parse_tool_id_invalid_formats() {
            // Test invalid formats
            assert!(desktop::parse_tool_id("nounderscore").is_err(), "Should fail for no underscore");
            assert!(desktop::parse_tool_id("_empty_skill").is_err(), "Should fail for empty skill ID");
            assert!(desktop::parse_tool_id("empty_tool_").is_err(), "Should fail for empty tool name");
            assert!(desktop::parse_tool_id("").is_err(), "Should fail for empty string");
        }

        #[test]
        fn test_convert_to_openai_schema() {
            // Test MCP schema to OpenAI conversion
            let mcp_schema = serde_json::json!({
                "properties": {
                    "owner": {"type": "string"},
                    "repo": {"type": "string"}
                },
                "required": ["owner", "repo"]
            });

            let openai_schema = desktop::convert_to_openai_schema(mcp_schema)
                .expect("Should convert MCP to OpenAI schema");

            assert_eq!(openai_schema["type"], "object");
            assert!(openai_schema["properties"].is_object());
            assert!(openai_schema["required"].is_array());

            // Test already OpenAI format (should pass through)
            let existing_openai = serde_json::json!({
                "type": "object",
                "properties": {"test": {"type": "string"}},
                "required": ["test"]
            });

            let result = desktop::convert_to_openai_schema(existing_openai.clone())
                .expect("Should handle existing OpenAI format");
            assert_eq!(result, existing_openai);
        }

        #[test]
        fn test_zeroclaw_format_compliance() {
            // Test that our ZeroClaw format matches expected OpenAI structure
            let schema = ZeroClawToolSchema {
                type_field: "function".to_string(),
                function: ZeroClawFunction {
                    name: "github_list_issues".to_string(),
                    description: "List GitHub issues for a repository".to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "owner": {"type": "string", "description": "Repository owner"},
                            "repo": {"type": "string", "description": "Repository name"},
                            "state": {"type": "string", "enum": ["open", "closed", "all"], "default": "open"}
                        },
                        "required": ["owner", "repo"]
                    })
                }
            };

            // Serialize and check format
            let json = serde_json::to_value(&schema).expect("Should serialize");

            // Check OpenAI compatibility
            assert_eq!(json["type"], "function");
            assert!(json["function"].is_object());
            assert!(json["function"]["name"].is_string());
            assert!(json["function"]["description"].is_string());
            assert!(json["function"]["parameters"].is_object());

            // Check parameter schema
            let params = &json["function"]["parameters"];
            assert_eq!(params["type"], "object");
            assert!(params["properties"].is_object());
            assert!(params["required"].is_array());
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    mod mobile_tests {
        use super::*;

        #[tokio::test]
        async fn test_mobile_stub_runtime_get_tool_schemas() {
            let result = mobile::runtime_get_tool_schemas().await;

            // Mobile should return empty list with helpful error
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("not available on mobile"));
        }

        #[tokio::test]
        async fn test_mobile_stub_runtime_execute_tool() {
            let result = mobile::runtime_execute_tool(
                "test_tool".to_string(),
                "{}".to_string()
            ).await;

            // Mobile should return error
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("not available on mobile"));
        }
    }

    #[test]
    fn test_zeroclaw_struct_defaults() {
        // Test that ZeroClaw structs can be created with serde_json
        let tool_schema: ZeroClawToolSchema = serde_json::from_value(serde_json::json!({
            "type": "function",
            "function": {
                "name": "test",
                "description": "test",
                "parameters": {}
            }
        })).expect("Should deserialize from JSON");

        assert_eq!(tool_schema.type_field, "function");
        assert_eq!(tool_schema.function.name, "test");

        // Test tool result
        let tool_result: ZeroClawToolResult = serde_json::from_value(serde_json::json!({
            "success": true,
            "output": "result",
            "error": null,
            "execution_time": 1000
        })).expect("Should deserialize tool result");

        assert!(tool_result.success);
        assert_eq!(tool_result.output, "result");
        assert_eq!(tool_result.execution_time, Some(1000));
    }

    #[test]
    fn test_error_handling_structures() {
        // Test that error scenarios can be properly serialized
        let error_result = ZeroClawToolResult {
            success: false,
            output: String::new(),
            error: Some("Connection timeout".to_string()),
            execution_time: Some(30000) // 30 second timeout
        };

        let json = serde_json::to_string(&error_result).expect("Should serialize error");
        assert!(json.contains("false"));
        assert!(json.contains("Connection timeout"));
        assert!(json.contains("30000"));

        // Test deserialization back
        let parsed: ZeroClawToolResult = serde_json::from_str(&json)
            .expect("Should parse error result");
        assert!(!parsed.success);
        assert_eq!(parsed.error, Some("Connection timeout".to_string()));
    }
}
