use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::mpsc;

use crate::openhuman::skills::cron_scheduler::CronScheduler;
use crate::openhuman::skills::skill_registry::SkillRegistry;
use crate::openhuman::skills::types::{SkillConfig, SkillMessage, SkillStatus, ToolDefinition};

/// Dependencies passed to a skill instance for bridge installation.
#[allow(dead_code)]
pub struct BridgeDeps {
    pub cron_scheduler: Arc<CronScheduler>,
    pub skill_registry: Arc<SkillRegistry>,
    pub memory_client: Option<crate::openhuman::memory::MemoryClientRef>,
    pub webhook_router: Option<Arc<crate::openhuman::webhooks::WebhookRouter>>,
    pub data_dir: PathBuf,
    // NOTE: No v8_creation_lock - QuickJS doesn't need it
}

/// Shared mutable state for a skill instance.
pub struct SkillState {
    pub status: SkillStatus,
    pub tools: Vec<ToolDefinition>,
    pub error: Option<String>,
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

/// A running skill instance using QuickJS.
pub struct QjsSkillInstance {
    pub config: SkillConfig,
    pub state: Arc<RwLock<SkillState>>,
    pub sender: mpsc::Sender<SkillMessage>,
    pub skill_dir: PathBuf,
    pub data_dir: PathBuf,
}
