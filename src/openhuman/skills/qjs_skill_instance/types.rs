//! Type definitions for the QuickJS skill instance.
//!
//! This module defines the core data structures used by the QuickJS skill runtime,
//! including shared state, dependencies, and the main instance handle.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::mpsc;

use crate::openhuman::skills::cron_scheduler::CronScheduler;
use crate::openhuman::skills::skill_registry::SkillRegistry;
use crate::openhuman::skills::types::{SkillConfig, SkillMessage, SkillStatus, ToolDefinition};

/// Dependencies passed to a skill instance during initialization.
///
/// These resources are used to set up the native "ops" (bridges) that allow
/// the JavaScript environment to interact with the host system.
pub struct BridgeDeps {
    /// Global scheduler for periodic tasks.
    pub cron_scheduler: Arc<CronScheduler>,
    /// Access to the global skill registry.
    pub skill_registry: Arc<SkillRegistry>,
    /// Client for interacting with the OpenHuman memory system.
    pub memory_client: Option<crate::openhuman::memory::MemoryClientRef>,
    /// Router for incoming webhooks targeted at skills.
    pub webhook_router: Option<Arc<crate::openhuman::webhooks::WebhookRouter>>,
    /// Base directory where the skill can store persistent data.
    pub data_dir: PathBuf,
}

/// Shared mutable state for a single skill instance.
///
/// This state is accessed by both the Rust host and the JavaScript runtime.
pub struct SkillState {
    /// Current lifecycle status (Initializing, Running, Error, etc.).
    pub status: SkillStatus,
    /// List of tools exposed by this skill to the LLM/system.
    pub tools: Vec<ToolDefinition>,
    /// Last error message, if status is `Error`.
    pub error: Option<String>,
    /// Arbitrary state published by the skill for UI or system use.
    pub published_state: HashMap<String, serde_json::Value>,
}

impl Default for SkillState {
    fn default() -> Self {
        Self {
            status: SkillStatus::Pending,
            tools: Vec::new(),
            error: None,
            published_state: HashMap::new(),
        }
    }
}

/// A handle to a running QuickJS skill instance.
///
/// It encapsulates the configuration, shared state, and a communication
/// channel to the instance's background task.
pub struct QjsSkillInstance {
    /// Static configuration for the skill (ID, name, entry point, etc.).
    pub config: SkillConfig,
    /// Thread-safe reference to the instance's mutable state.
    pub state: Arc<RwLock<SkillState>>,
    /// Channel used to send messages (e.g., sync, tool calls) to the background task.
    pub sender: mpsc::Sender<SkillMessage>,
    /// Path to the directory containing the skill's source bundle.
    pub skill_dir: PathBuf,
    /// Path to the directory where the skill stores its persistent data.
    pub data_dir: PathBuf,
}
