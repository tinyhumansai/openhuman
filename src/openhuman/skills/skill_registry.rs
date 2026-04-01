//! SkillRegistry — tracks all registered/running skills and routes messages.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::{mpsc, oneshot};

use crate::openhuman::skills::qjs_skill_instance::SkillState;
use crate::openhuman::skills::types::{
    SkillConfig, SkillMessage, SkillSnapshot, SkillStatus, ToolCallOrigin, ToolDefinition,
    ToolResult,
};

/// Entry in the registry for a single skill.
struct RegistryEntry {
    /// Sender to the skill's message loop.
    sender: mpsc::Sender<SkillMessage>,
    /// Shared state readable without going through the message loop.
    state: Arc<RwLock<SkillState>>,
    /// Config from the manifest.
    config: SkillConfig,
    /// Handle to the spawned Tokio task.
    #[allow(dead_code)]
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

/// Central registry of all skills known to the runtime.
pub struct SkillRegistry {
    skills: RwLock<HashMap<String, RegistryEntry>>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: RwLock::new(HashMap::new()),
        }
    }

    /// Register a skill instance after it has been created and spawned.
    pub fn register(
        &self,
        skill_id: &str,
        config: SkillConfig,
        sender: mpsc::Sender<SkillMessage>,
        state: Arc<RwLock<SkillState>>,
        task_handle: tokio::task::JoinHandle<()>,
    ) {
        log::info!("[runtime] registering skill '{}'", skill_id);
        self.skills.write().insert(
            skill_id.to_string(),
            RegistryEntry {
                sender,
                state,
                config,
                task_handle: Some(task_handle),
            },
        );
    }

    /// Unregister a skill (after it has stopped).
    pub fn unregister(&self, skill_id: &str) {
        self.skills.write().remove(skill_id);
    }

    /// Get a snapshot of all registered skills.
    pub fn list_skills(&self) -> Vec<SkillSnapshot> {
        self.skills
            .read()
            .iter()
            .map(|(skill_id, entry)| {
                let state = entry.state.read();
                Self::build_snapshot(skill_id, entry, &state)
            })
            .collect()
    }

    /// Get a snapshot of a single skill.
    pub fn get_skill(&self, skill_id: &str) -> Option<SkillSnapshot> {
        let skills = self.skills.read();
        skills.get(skill_id).map(|entry| {
            let state = entry.state.read();
            Self::build_snapshot(skill_id, entry, &state)
        })
    }

    fn build_snapshot(skill_id: &str, entry: &RegistryEntry, state: &SkillState) -> SkillSnapshot {
        use crate::openhuman::skills::types::derive_connection_status;

        // setup_complete is populated later by the caller who has access to PreferencesStore
        let setup_complete = false;
        let connection_status =
            derive_connection_status(state.status, setup_complete, &state.published_state);

        SkillSnapshot {
            skill_id: skill_id.to_string(),
            name: entry.config.name.clone(),
            status: state.status,
            tools: state.tools.clone(),
            error: state.error.clone(),
            state: state.published_state.clone(),
            setup_complete,
            connection_status,
        }
    }

    /// Get the status of a skill.
    #[allow(dead_code)]
    pub fn get_status(&self, skill_id: &str) -> Option<SkillStatus> {
        self.skills
            .read()
            .get(skill_id)
            .map(|e| e.state.read().status)
    }

    /// Call a tool on a specific skill from external host surfaces.
    pub async fn call_tool(
        &self,
        skill_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolResult, String> {
        self.call_tool_scoped(ToolCallOrigin::External, skill_id, tool_name, arguments)
            .await
    }

    /// Call a tool with an explicit origin so runtime policy can enforce boundaries.
    pub async fn call_tool_scoped(
        &self,
        origin: ToolCallOrigin,
        skill_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolResult, String> {
        if let ToolCallOrigin::SkillSelf {
            skill_id: caller_skill_id,
        } = &origin
        {
            if caller_skill_id != skill_id {
                return Err(format!(
                    "Cross-skill tool calls are forbidden: '{}' cannot call '{}.{}'",
                    caller_skill_id, skill_id, tool_name
                ));
            }
        }

        log::info!(
            "[skill:{}] call_tool '{}' — dispatching to event loop",
            skill_id,
            tool_name
        );
        let sender = {
            let skills = self.skills.read();
            let entry = skills
                .get(skill_id)
                .ok_or_else(|| format!("Skill '{}' not found", skill_id))?;
            let status = entry.state.read().status;
            if status != SkillStatus::Running {
                log::warn!(
                    "[skill:{}] call_tool '{}' — skill not running (status: {:?})",
                    skill_id,
                    tool_name,
                    status
                );
                return Err(format!(
                    "Skill '{}' is not running (status: {:?})",
                    skill_id, status
                ));
            }
            entry.sender.clone()
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        sender
            .send(SkillMessage::CallTool {
                tool_name: tool_name.to_string(),
                arguments,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                log::error!(
                    "[skill:{}] call_tool '{}' — message channel closed",
                    skill_id,
                    tool_name
                );
                format!("Skill '{}' message channel closed", skill_id)
            })?;

        log::debug!(
            "[skill:{}] call_tool '{}' — message sent, waiting for reply",
            skill_id,
            tool_name
        );

        match reply_rx.await {
            Ok(result) => {
                log::info!(
                    "[skill:{}] call_tool '{}' — got reply (is_err: {})",
                    skill_id,
                    tool_name,
                    result.is_err()
                );
                result
            }
            Err(_) => {
                log::error!(
                    "[skill:{}] call_tool '{}' — reply channel dropped (skill event loop may have crashed or tool timed out)",
                    skill_id,
                    tool_name
                );
                Err(format!(
                    "Skill '{}' did not respond to tool call '{}'",
                    skill_id, tool_name
                ))
            }
        }
    }

    /// Send a server event to all running skills.
    pub async fn broadcast_event(&self, event: &str, data: serde_json::Value) {
        let senders: Vec<mpsc::Sender<SkillMessage>> = {
            self.skills
                .read()
                .values()
                .filter(|e| e.state.read().status == SkillStatus::Running)
                .map(|e| e.sender.clone())
                .collect()
        };

        for sender in senders {
            let _ = sender
                .send(SkillMessage::ServerEvent {
                    event: event.to_string(),
                    data: data.clone(),
                })
                .await;
        }
    }

    /// Send a cron trigger to a specific skill.
    #[allow(dead_code)]
    pub async fn trigger_cron(&self, skill_id: &str, schedule_id: &str) -> Result<(), String> {
        let sender = {
            let skills = self.skills.read();
            let entry = skills
                .get(skill_id)
                .ok_or_else(|| format!("Skill '{}' not found", skill_id))?;
            entry.sender.clone()
        };

        sender
            .send(SkillMessage::CronTrigger {
                schedule_id: schedule_id.to_string(),
            })
            .await
            .map_err(|_| format!("Skill '{}' message channel closed", skill_id))
    }

    /// Stop a specific skill gracefully.
    pub async fn stop_skill(&self, skill_id: &str) -> Result<(), String> {
        let sender = {
            let skills = self.skills.read();
            let entry = skills
                .get(skill_id)
                .ok_or_else(|| format!("Skill '{}' not found", skill_id))?;

            // Update status
            entry.state.write().status = SkillStatus::Stopping;
            entry.sender.clone()
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        sender
            .send(SkillMessage::Stop { reply: reply_tx })
            .await
            .map_err(|_| format!("Skill '{}' message channel closed", skill_id))?;

        // Wait for the skill to acknowledge stopping
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), reply_rx).await;

        // Don't unregister — the skill stays in the registry with Stopped status
        // so the UI can still query it and allow restart without full rediscovery.
        Ok(())
    }

    /// Get all tool definitions across all running skills.
    /// Returns tuples of (skill_id, tool_definition).
    pub fn all_tools(&self) -> Vec<(String, ToolDefinition)> {
        self.skills
            .read()
            .iter()
            .filter(|(_, entry)| entry.state.read().status == SkillStatus::Running)
            .flat_map(|(skill_id, entry)| {
                let state = entry.state.read();
                state
                    .tools
                    .iter()
                    .map(|t| (skill_id.clone(), t.clone()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Check if a skill is registered.
    pub fn has_skill(&self, skill_id: &str) -> bool {
        self.skills.read().contains_key(skill_id)
    }

    /// Send a message to a specific skill's message loop.
    /// Returns an error if the skill is not registered or the channel is full.
    pub fn send_message(&self, skill_id: &str, msg: SkillMessage) -> Result<(), String> {
        log::info!("[runtime] sending message to '{}': {:?}", skill_id, msg);
        let sender = {
            let skills = self.skills.read();
            let entry = skills
                .get(skill_id)
                .ok_or_else(|| format!("Skill '{}' not found", skill_id))?;
            let status = entry.state.read().status;
            if status != SkillStatus::Running {
                return Err(format!(
                    "Skill '{}' is not running (status: {:?})",
                    skill_id, status
                ));
            }
            entry.sender.clone()
        };

        match sender.try_send(msg) {
            Ok(()) => {
                log::info!(
                    "[runtime] Successfully sent message to skill '{}'",
                    skill_id
                );
                Ok(())
            }
            Err(e) => {
                let error_msg = format!("Failed to send message to skill '{}': {e}", skill_id);
                log::error!("[runtime] {}", error_msg);
                Err(error_msg)
            }
        }
    }

    /// Send an incoming webhook request to a specific skill and wait for the response.
    ///
    /// Returns the skill's response (status code, headers, body) or an error.
    /// Times out after 25 seconds (under the backend's 30-second timeout).
    pub async fn send_webhook_request(
        &self,
        skill_id: &str,
        correlation_id: String,
        method: String,
        path: String,
        headers: std::collections::HashMap<String, serde_json::Value>,
        query: std::collections::HashMap<String, String>,
        body: String,
        tunnel_id: String,
        tunnel_name: String,
    ) -> Result<crate::openhuman::webhooks::WebhookResponseData, String> {
        let sender = {
            let skills = self.skills.read();
            let entry = skills
                .get(skill_id)
                .ok_or_else(|| format!("Skill '{}' not found", skill_id))?;
            let status = entry.state.read().status;
            if status != SkillStatus::Running {
                return Err(format!(
                    "Skill '{}' is not running (status: {:?})",
                    skill_id, status
                ));
            }
            entry.sender.clone()
        };

        let (reply_tx, reply_rx) = oneshot::channel();

        sender
            .send(SkillMessage::WebhookRequest {
                correlation_id: correlation_id.clone(),
                method,
                path,
                headers,
                query,
                body,
                tunnel_id,
                tunnel_name,
                reply: reply_tx,
            })
            .await
            .map_err(|_| format!("Skill '{}' message channel closed", skill_id))?;

        match tokio::time::timeout(std::time::Duration::from_secs(25), reply_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(format!(
                "Skill '{}' webhook reply channel dropped",
                skill_id
            )),
            Err(_) => Err(format!(
                "Skill '{}' webhook handler timed out (25s)",
                skill_id
            )),
        }
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::skills::types::{SkillConfig, SkillStatus, ToolCallOrigin};

    fn register_running_skill(registry: &SkillRegistry, skill_id: &str) {
        let (tx, mut rx) = mpsc::channel(8);
        let state = Arc::new(RwLock::new(SkillState {
            status: SkillStatus::Running,
            tools: vec![],
            error: None,
            published_state: HashMap::new(),
        }));
        let task = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let SkillMessage::CallTool { reply, .. } = msg {
                    let _ = reply.send(Ok(ToolResult {
                        content: vec![],
                        is_error: false,
                    }));
                }
            }
        });

        registry.register(
            skill_id,
            SkillConfig {
                skill_id: skill_id.to_string(),
                name: skill_id.to_string(),
                entry_point: "index.js".to_string(),
                memory_limit: 1024,
                auto_start: false,
            },
            tx,
            state,
            task,
        );
    }

    #[tokio::test]
    async fn external_origin_can_call_any_skill_tool() {
        let registry = SkillRegistry::new();
        register_running_skill(&registry, "alpha");

        let result = registry
            .call_tool_scoped(
                ToolCallOrigin::External,
                "alpha",
                "echo",
                serde_json::json!({}),
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn skill_origin_cannot_call_other_skill_tool() {
        let registry = SkillRegistry::new();
        register_running_skill(&registry, "alpha");
        register_running_skill(&registry, "beta");

        let err = registry
            .call_tool_scoped(
                ToolCallOrigin::SkillSelf {
                    skill_id: "alpha".to_string(),
                },
                "beta",
                "echo",
                serde_json::json!({}),
            )
            .await
            .expect_err("cross-skill call should be denied");

        assert!(err.contains("Cross-skill tool calls are forbidden"));
    }

    #[tokio::test]
    async fn skill_origin_can_call_own_tool() {
        let registry = SkillRegistry::new();
        register_running_skill(&registry, "alpha");

        let result = registry
            .call_tool_scoped(
                ToolCallOrigin::SkillSelf {
                    skill_id: "alpha".to_string(),
                },
                "alpha",
                "echo",
                serde_json::json!({}),
            )
            .await;

        assert!(result.is_ok());
    }
}
