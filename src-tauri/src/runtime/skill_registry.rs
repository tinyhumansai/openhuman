//! SkillRegistry — tracks all registered/running skills and routes messages.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::{mpsc, oneshot};

use crate::runtime::v8_skill_instance::SkillState;
use crate::runtime::types::{
    SkillConfig, SkillMessage, SkillSnapshot, SkillStatus, ToolDefinition, ToolResult,
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
                SkillSnapshot {
                    skill_id: skill_id.clone(),
                    name: entry.config.name.clone(),
                    status: state.status,
                    tools: state.tools.clone(),
                    error: state.error.clone(),
                    state: state.published_state.clone(),
                }
            })
            .collect()
    }

    /// Get a snapshot of a single skill.
    pub fn get_skill(&self, skill_id: &str) -> Option<SkillSnapshot> {
        let skills = self.skills.read();
        skills.get(skill_id).map(|entry| {
            let state = entry.state.read();
            SkillSnapshot {
                skill_id: skill_id.to_string(),
                name: entry.config.name.clone(),
                status: state.status,
                tools: state.tools.clone(),
                error: state.error.clone(),
                state: state.published_state.clone(),
            }
        })
    }

    /// Get the status of a skill.
    #[allow(dead_code)]
    pub fn get_status(&self, skill_id: &str) -> Option<SkillStatus> {
        self.skills
            .read()
            .get(skill_id)
            .map(|e| e.state.read().status)
    }

    /// Call a tool on a specific skill. Returns a oneshot receiver for the result.
    pub async fn call_tool(
        &self,
        skill_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolResult, String> {
        let sender = {
            let skills = self.skills.read();
            let entry = skills
                .get(skill_id)
                .ok_or_else(|| format!("Skill '{}' not found", skill_id))?;
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
            .map_err(|_| format!("Skill '{}' message channel closed", skill_id))?;

        reply_rx
            .await
            .map_err(|_| format!("Skill '{}' did not respond to tool call", skill_id))?
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

        // Remove from registry
        self.unregister(skill_id);
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
            entry.sender.clone()
        };

        match sender.try_send(msg) {
            Ok(()) => {
                log::info!("[runtime] Successfully sent message to skill '{}'", skill_id);
                Ok(())
            }
            Err(e) => {
                let error_msg = format!("Failed to send message to skill '{}': {e}", skill_id);
                log::error!("[runtime] {}", error_msg);
                Err(error_msg)
            }
        }
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}
