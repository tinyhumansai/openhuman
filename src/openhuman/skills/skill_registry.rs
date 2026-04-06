//! SkillRegistry — tracks all registered/running skills and routes messages.
//! 
//! The registry is the central source of truth for which skills are currently
//! active in the runtime. It manages the communication channels (senders) to 
//! each skill's event loop and provides methods for interacting with them.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::{mpsc, oneshot};

use crate::openhuman::skills::qjs_skill_instance::SkillState;
use crate::openhuman::skills::types::{
    self, SkillConfig, SkillMessage, SkillSnapshot, SkillStatus, ToolCallOrigin, ToolDefinition,
    ToolResult,
};

/// Internal entry in the registry representing a single skill instance.
struct RegistryEntry {
    /// Sender to the skill's message loop for asynchronous communication.
    sender: mpsc::Sender<SkillMessage>,
    /// Shared state of the skill, readable without going through the message loop.
    state: Arc<RwLock<SkillState>>,
    /// Configuration of the skill, derived from its manifest.
    config: SkillConfig,
    /// Handle to the spawned Tokio task running the skill's event loop.
    #[allow(dead_code)]
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

/// Central registry of all skills known to the runtime.
/// 
/// This struct provides thread-safe access to all active skills and mediates
/// message routing, tool calls, and event broadcasting.
pub struct SkillRegistry {
    /// Map of skill IDs to their registry entries, protected by a read-write lock.
    skills: RwLock<HashMap<String, RegistryEntry>>,
}

impl SkillRegistry {
    /// Create a new, empty SkillRegistry.
    pub fn new() -> Self {
        Self {
            skills: RwLock::new(HashMap::new()),
        }
    }

    /// Register a skill instance after it has been created and spawned.
    /// 
    /// This makes the skill available for discovery and interaction through the registry.
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

    /// Unregister a skill from the registry.
    /// 
    /// This is typically called after a skill has successfully stopped.
    pub fn unregister(&self, skill_id: &str) {
        self.skills.write().remove(skill_id);
    }

    /// Get a snapshot of all currently registered skills.
    /// 
    /// Returns a list of `SkillSnapshot` containing the latest state of each skill.
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

    /// Get a snapshot of a single skill by its ID.
    /// 
    /// Returns `None` if the skill is not found in the registry.
    pub fn get_skill(&self, skill_id: &str) -> Option<SkillSnapshot> {
        let skills = self.skills.read();
        skills.get(skill_id).map(|entry| {
            let state = entry.state.read();
            Self::build_snapshot(skill_id, entry, &state)
        })
    }

    /// Helper function to build a `SkillSnapshot` from a `RegistryEntry` and its current `SkillState`.
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

    /// Get the current status of a specific skill.
    #[allow(dead_code)]
    pub fn get_status(&self, skill_id: &str) -> Option<SkillStatus> {
        self.skills
            .read()
            .get(skill_id)
            .map(|e| e.state.read().status)
    }

    /// Call a tool on a specific skill from an external host surface (e.g., UI, CLI).
    pub async fn call_tool(
        &self,
        skill_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolResult, String> {
        self.call_tool_scoped(ToolCallOrigin::External, skill_id, tool_name, arguments)
            .await
    }

    /// Call a tool with an explicit origin so runtime policy can enforce security boundaries.
    /// 
    /// Currently, cross-skill tool calls are forbidden to ensure isolation.
    pub async fn call_tool_scoped(
        &self,
        origin: ToolCallOrigin,
        skill_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolResult, String> {
        // Enforce isolation: A skill cannot call tools belonging to another skill.
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

        // Retrieve the sender for the target skill's event loop.
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

        // Create a one-shot channel to receive the tool result from the skill's event loop.
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

        // Wait for the skill to process the tool call and return a result.
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

    /// Send a server event to all skills that are currently in the `Running` state.
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

    /// Send a cron trigger message to a specific skill.
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

    /// Stop a specific skill gracefully by sending a `Stop` message.
    /// 
    /// It waits up to 5 seconds for the skill to acknowledge the stop request.
    pub async fn stop_skill(&self, skill_id: &str) -> Result<(), String> {
        let sender = {
            let skills = self.skills.read();
            let entry = skills
                .get(skill_id)
                .ok_or_else(|| format!("Skill '{}' not found", skill_id))?;

            // Transition the skill state to `Stopping`.
            entry.state.write().status = SkillStatus::Stopping;
            entry.sender.clone()
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        sender
            .send(SkillMessage::Stop { reply: reply_tx })
            .await
            .map_err(|_| format!("Skill '{}' message channel closed", skill_id))?;

        // Wait for the skill to acknowledge stopping, with a 5-second timeout.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), reply_rx).await;

        // Note: The skill is NOT unregistered here. It remains in the registry
        // with `Stopped` status, allowing it to be restarted without full discovery.
        Ok(())
    }

    /// Get all tool definitions across all running skills.
    /// 
    /// Returns a list of tuples containing the skill ID and the tool definition.
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

    /// Check if a skill with the given ID is present in the registry.
    pub fn has_skill(&self, skill_id: &str) -> bool {
        self.skills.read().contains_key(skill_id)
    }

    /// Merge a patch into a running skill's published state and broadcast the change.
    /// 
    /// This is used by components like the ping scheduler to update health information.
    pub async fn merge_published_state(
        &self,
        skill_id: &str,
        patch: HashMap<String, serde_json::Value>,
    ) -> Result<(), String> {
        {
            let skills = self.skills.read();
            let entry = skills
                .get(skill_id)
                .ok_or_else(|| format!("Skill '{}' not found", skill_id))?;
            let mut state = entry.state.write();
            if state.status != SkillStatus::Running {
                return Err(format!(
                    "Skill '{}' is not running (status: {:?})",
                    skill_id, state.status
                ));
            }
            for (k, v) in patch {
                state.published_state.insert(k, v);
            }
        }
        // Notify the rest of the system that the skill's state has changed.
        self.broadcast_event(
            types::events::SKILL_STATE_CHANGED,
            serde_json::json!({ "skill_id": skill_id }),
        )
        .await;
        Ok(())
    }

    /// Send a raw `SkillMessage` to a specific skill's message loop.
    /// 
    /// This is a non-blocking operation that returns an error if the skill
    /// is not running or the message channel is full/closed.
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

    /// Send an incoming webhook request to a specific skill and wait for its response.
    ///
    /// This routes a request from an external tunnel to the skill's internal 
    /// webhook handler. It returns the response (status, headers, body) or an error.
    /// 
    /// The request will time out after 25 seconds if no response is received.
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
                correlation_id,
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

        // Wait for the skill to respond, with a 25-second timeout.
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
    /// Create a default, empty SkillRegistry.
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
