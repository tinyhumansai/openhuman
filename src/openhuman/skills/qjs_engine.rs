//! RuntimeEngine — top-level orchestrator for the QuickJS skill runtime.
//!
//! Manages skill lifecycle and provides the public API consumed by Tauri commands.
//! Uses QuickJS (via rquickjs) for JavaScript execution.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use tauri::{AppHandle, Emitter};

/// Global RuntimeEngine instance. Uses `RwLock` so it can be swapped in tests.
static GLOBAL_ENGINE: parking_lot::RwLock<Option<Arc<RuntimeEngine>>> =
    parking_lot::RwLock::new(None);

/// Store a reference to the RuntimeEngine so RPC handlers can access it.
/// In production, call this once during app setup. In tests, can be called
/// multiple times (each call replaces the previous engine).
pub fn set_global_engine(engine: Arc<RuntimeEngine>) {
    *GLOBAL_ENGINE.write() = Some(engine);
}

/// Get a clone of the global RuntimeEngine Arc.
/// Returns `None` if the engine has not been initialized yet.
pub fn global_engine() -> Option<Arc<RuntimeEngine>> {
    GLOBAL_ENGINE.read().clone()
}

/// Get the global RuntimeEngine or return an error string.
pub fn require_engine() -> Result<Arc<RuntimeEngine>, String> {
    global_engine().ok_or_else(|| "skill runtime not initialized".to_string())
}

use crate::openhuman::skills::cron_scheduler::CronScheduler;
use crate::openhuman::skills::manifest::SkillManifest;
use crate::openhuman::skills::ping_scheduler::PingScheduler;
use crate::openhuman::skills::preferences::PreferencesStore;
use crate::openhuman::skills::qjs_skill_instance::{BridgeDeps, QjsSkillInstance};
use crate::openhuman::skills::skill_registry::SkillRegistry;
use crate::openhuman::skills::socket_manager::SocketManager;
use crate::openhuman::skills::types::{events, SkillSnapshot, SkillStatus, ToolResult};
// IdbStorage removed during runtime cleanup

/// The central runtime engine using QuickJS.
pub struct RuntimeEngine {
    /// Registry of all skills.
    registry: Arc<SkillRegistry>,
    /// Global cron scheduler for timed skill triggers.
    cron_scheduler: Arc<CronScheduler>,
    /// Background ping scheduler for skill health checks.
    ping_scheduler: Arc<PingScheduler>,
    /// Persistent user enable/disable preferences for skills.
    preferences: Arc<PreferencesStore>,
    /// Base data directory for skills (platform-aware).
    skills_data_dir: PathBuf,
    /// Directory containing skill source files.
    skills_source_dir: RwLock<Option<PathBuf>>,
    /// Tauri resource directory (bundled skills in production).
    resource_dir: RwLock<Option<PathBuf>>,
    /// Tauri app handle for emitting events.
    app_handle: RwLock<Option<AppHandle>>,
    /// Socket manager for emitting tool:sync events.
    socket_manager: RwLock<Option<Arc<SocketManager>>>,
    /// Workspace directory for user-installed skills from registry.
    workspace_dir: RwLock<Option<PathBuf>>,
}

impl RuntimeEngine {
    /// Create a new RuntimeEngine.
    pub fn new(skills_data_dir: PathBuf) -> Result<Self, String> {
        let registry = Arc::new(SkillRegistry::new());
        let cron_scheduler = Arc::new(CronScheduler::new());
        cron_scheduler.set_registry(Arc::clone(&registry));
        let ping_scheduler = Arc::new(PingScheduler::new());
        ping_scheduler.set_registry(Arc::clone(&registry));
        let preferences = Arc::new(PreferencesStore::new(&skills_data_dir));

        log::info!("[runtime] QuickJS RuntimeEngine created");

        Ok(Self {
            registry,
            cron_scheduler,
            ping_scheduler,
            preferences,
            skills_data_dir,
            skills_source_dir: RwLock::new(None),
            resource_dir: RwLock::new(None),
            app_handle: RwLock::new(None),
            socket_manager: RwLock::new(None),
            workspace_dir: RwLock::new(None),
        })
    }

    /// Get a clone of the skill registry Arc.
    pub fn registry(&self) -> Arc<SkillRegistry> {
        Arc::clone(&self.registry)
    }

    /// Get a clone of the cron scheduler Arc.
    pub fn cron_scheduler(&self) -> Arc<CronScheduler> {
        Arc::clone(&self.cron_scheduler)
    }

    /// Get a clone of the ping scheduler Arc.
    pub fn ping_scheduler(&self) -> Arc<PingScheduler> {
        Arc::clone(&self.ping_scheduler)
    }

    /// Set the Tauri app handle for emitting events to the frontend.
    pub fn set_app_handle(&self, handle: AppHandle) {
        self.ping_scheduler.set_app_handle(handle.clone());
        *self.app_handle.write() = Some(handle);
    }

    /// Set the directory containing skill source files.
    #[allow(dead_code)]
    pub fn set_skills_source_dir(&self, dir: PathBuf) {
        *self.skills_source_dir.write() = Some(dir);
    }

    /// Set the Tauri resource directory (for bundled skills in production).
    pub fn set_resource_dir(&self, dir: PathBuf) {
        log::info!("[runtime] Resource directory set to: {:?}", dir);
        *self.resource_dir.write() = Some(dir);
    }

    /// Set the socket manager for emitting `tool:sync` events.
    pub fn set_socket_manager(&self, mgr: Arc<SocketManager>) {
        *self.socket_manager.write() = Some(mgr);
    }

    /// Set the workspace directory for user-installed skills from the registry.
    pub fn set_workspace_dir(&self, dir: PathBuf) {
        log::info!("[runtime] Workspace directory set to: {:?}", dir);
        *self.workspace_dir.write() = Some(dir);
    }

    /// Notify the backend of the current tool state via `tool:sync`.
    async fn sync_tools(&self) {
        let mgr = { self.socket_manager.read().clone() };
        if let Some(mgr) = mgr {
            mgr.sync_tools().await;
        }
    }

    /// Get the skills source directory.
    fn get_skills_source_dir(&self) -> Result<PathBuf, String> {
        // 1. Explicitly set source dir (highest priority)
        if let Some(dir) = self.skills_source_dir.read().as_ref() {
            log::info!("[runtime] Using explicit skills source dir: {:?}", dir);
            return Ok(dir.clone());
        }

        let current =
            std::env::current_dir().map_err(|e| format!("Failed to get current dir: {e}"))?;

        // 2. Dev: cwd/skills/skills
        let dev_skills = current.join("skills").join("skills");
        if dev_skills.exists() {
            log::info!("[runtime] Using dev skills dir: {:?}", dev_skills);
            return Ok(dev_skills);
        }

        // 3. Dev: ../skills/skills
        if let Some(parent) = current.parent() {
            let parent_skills = parent.join("skills").join("skills");
            if parent_skills.exists() {
                log::info!("[runtime] Using parent dev skills dir: {:?}", parent_skills);
                return Ok(parent_skills);
            }
        }

        // 4. Production: bundled resources
        if let Some(resource_dir) = self.resource_dir.read().as_ref() {
            let bundled_skills = resource_dir.join("_up_").join("skills").join("skills");
            if bundled_skills.exists() {
                log::info!(
                    "[runtime] Using bundled skills from resources: {:?}",
                    bundled_skills
                );
                return Ok(bundled_skills);
            }

            let bundled_skills_alt = resource_dir.join("skills");
            if bundled_skills_alt.exists() {
                log::info!(
                    "[runtime] Using bundled skills from resources (alt): {:?}",
                    bundled_skills_alt
                );
                return Ok(bundled_skills_alt);
            }

            log::warn!(
                "[runtime] Resource dir set but skills not found. Checked: {:?} and {:?}",
                bundled_skills,
                bundled_skills_alt
            );
        }

        // 5. Final fallback: app data dir
        let prod_dir = self.skills_data_dir.clone();
        log::info!(
            "[runtime] Skills source dir (data dir fallback): {:?}",
            prod_dir
        );
        Ok(prod_dir)
    }

    /// Expose the resolved skills source directory (for external callers like unified registry).
    pub fn skills_source_dir(&self) -> Result<PathBuf, String> {
        self.get_skills_source_dir()
    }

    /// Discover all JavaScript skills from the skills source directory and workspace.
    pub async fn discover_skills(&self) -> Result<Vec<SkillManifest>, String> {
        let mut manifests = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        // 1. Scan bundled/dev skills source directory
        let skills_dir = self.get_skills_source_dir()?;
        if skills_dir.exists() {
            self.scan_skills_dir(&skills_dir, &mut manifests, &mut seen_ids)
                .await?;
        }

        // 2. Scan workspace skills directory (user-installed from registry)
        if let Some(workspace_dir) = self.workspace_dir.read().as_ref() {
            let workspace_skills = workspace_dir.join("skills");
            if workspace_skills.exists() {
                log::info!(
                    "[runtime] Also scanning workspace skills dir: {:?}",
                    workspace_skills
                );
                self.scan_skills_dir(&workspace_skills, &mut manifests, &mut seen_ids)
                    .await?;
            }
        }

        Ok(manifests)
    }

    /// Scan a single directory for skill manifests, skipping already-seen IDs.
    async fn scan_skills_dir(
        &self,
        dir: &std::path::Path,
        manifests: &mut Vec<SkillManifest>,
        seen_ids: &mut std::collections::HashSet<String>,
    ) -> Result<(), String> {
        let mut entries = tokio::fs::read_dir(dir)
            .await
            .map_err(|e| format!("Failed to read skills dir {:?}: {e}", dir))?;

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                let manifest_path = path.join("manifest.json");
                if manifest_path.exists() {
                    match SkillManifest::from_path(&manifest_path).await {
                        Ok(manifest)
                            if manifest.is_javascript() && manifest.supports_current_platform() =>
                        {
                            if seen_ids.contains(&manifest.id) {
                                log::debug!(
                                    "[runtime] Skipping duplicate skill '{}' from {:?}",
                                    manifest.id,
                                    dir
                                );
                                continue;
                            }
                            log::info!(
                                "[runtime] Discovered skill '{}': {}",
                                manifest.id,
                                manifest.name
                            );
                            seen_ids.insert(manifest.id.clone());
                            manifests.push(manifest);
                        }
                        Ok(manifest) if manifest.is_javascript() => {
                            log::info!(
                                "[runtime] Skipping skill '{}': not supported on this platform",
                                manifest.id
                            );
                        }
                        Ok(_) => {
                            log::info!(
                                "[runtime] Skipping skill '{}': not a JavaScript skill",
                                manifest_path.display()
                            );
                        }
                        Err(e) => {
                            log::warn!("Failed to parse manifest at {:?}: {e}", manifest_path);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Start a specific skill by its ID.
    pub async fn start_skill(&self, skill_id: &str) -> Result<SkillSnapshot, String> {
        // Check if already running
        if self.registry.has_skill(skill_id) {
            if let Some(snap) = self.registry.get_skill(skill_id) {
                if snap.status == SkillStatus::Running || snap.status == SkillStatus::Initializing {
                    return Ok(snap);
                }
                self.registry.unregister(skill_id);
            }
        }

        // Look in bundled/dev source dir first, then workspace
        let skills_dir = self.get_skills_source_dir()?;
        let mut skill_dir = skills_dir.join(skill_id);
        let mut manifest_path = skill_dir.join("manifest.json");

        if !manifest_path.exists() {
            // Try workspace skills directory
            if let Some(workspace_dir) = self.workspace_dir.read().as_ref() {
                let ws_skill_dir = workspace_dir.join("skills").join(skill_id);
                let ws_manifest = ws_skill_dir.join("manifest.json");
                if ws_manifest.exists() {
                    log::info!("[runtime] Found skill '{}' in workspace dir", skill_id);
                    skill_dir = ws_skill_dir;
                    manifest_path = ws_manifest;
                }
            }
        }

        if !manifest_path.exists() {
            return Err(format!("Skill '{}' not found (no manifest.json)", skill_id));
        }

        let manifest = SkillManifest::from_path(&manifest_path).await?;
        if !manifest.is_javascript() {
            return Err(format!(
                "Skill '{}' uses runtime '{}', not a supported JavaScript runtime",
                skill_id, manifest.runtime
            ));
        }

        let config = manifest.to_config();
        let data_dir = self.skills_data_dir.join(skill_id);

        // Create the QuickJS skill instance
        log::info!(
            "[runtime] Creating QuickJS skill instance for '{}'",
            skill_id
        );
        log::info!("[runtime] Config: {:?}", config);
        log::info!("[runtime] Skill dir: {:?}", skill_dir);
        log::info!("[runtime] Data dir: {:?}", data_dir);
        let (instance, rx) = QjsSkillInstance::new(config.clone(), skill_dir, data_dir.clone());
        log::info!(
            "[runtime] QuickJS skill instance created for '{}'",
            skill_id
        );

        // Bundle bridge dependencies (no creation lock needed for QuickJS)
        let deps = BridgeDeps {
            cron_scheduler: self.cron_scheduler.clone(),
            skill_registry: self.registry.clone(),
            app_handle: self.app_handle.read().clone(),
            data_dir: data_dir.clone(),
        };

        // Spawn the skill's execution loop
        let task_handle = instance.spawn(rx, deps);

        // Register in the registry
        self.registry.register(
            skill_id,
            config,
            instance.sender.clone(),
            instance.state.clone(),
            task_handle,
        );

        self.emit_status_change(skill_id);

        // Wait for initialization to complete
        let state = instance.state.clone();
        let skill_id_owned = skill_id.to_string();
        let max_wait = std::time::Duration::from_secs(10);
        let poll_interval = std::time::Duration::from_millis(50);
        let start = std::time::Instant::now();

        loop {
            let current_status = state.read().status;

            match current_status {
                SkillStatus::Running => {
                    self.emit_status_change(&skill_id_owned);
                    self.sync_tools().await;
                    return Ok(instance.snapshot());
                }
                SkillStatus::Error => {
                    let error_msg = state
                        .read()
                        .error
                        .clone()
                        .unwrap_or_else(|| "Unknown initialization error".to_string());
                    // Don't unregister — keep the skill with Error status so the
                    // UI can see what happened and allow restart.
                    self.emit_status_change(&skill_id_owned);
                    return Err(format!(
                        "Skill '{}' failed to start: {}",
                        skill_id_owned, error_msg
                    ));
                }
                SkillStatus::Stopped => {
                    // Don't unregister — keep the skill with Stopped status so the
                    // UI can still query it and allow restart.
                    self.emit_status_change(&skill_id_owned);
                    return Err(format!(
                        "Skill '{}' stopped unexpectedly during initialization",
                        skill_id_owned
                    ));
                }
                SkillStatus::Initializing | SkillStatus::Pending => {
                    if start.elapsed() > max_wait {
                        log::warn!(
                            "[runtime] Skill '{}' initialization timeout, returning current state",
                            skill_id_owned
                        );
                        return Ok(instance.snapshot());
                    }
                    tokio::time::sleep(poll_interval).await;
                }
                SkillStatus::Stopping => {
                    return Err(format!(
                        "Skill '{}' is in unexpected Stopping state during startup",
                        skill_id_owned
                    ));
                }
            }
        }
    }

    /// Stop a running skill.
    pub async fn stop_skill(&self, skill_id: &str) -> Result<(), String> {
        self.registry.stop_skill(skill_id).await?;
        self.cron_scheduler.unregister_all_for_skill(skill_id);
        self.emit_status_change(skill_id);
        self.sync_tools().await;
        Ok(())
    }

    /// List all registered skills.
    pub fn list_skills(&self) -> Vec<SkillSnapshot> {
        self.registry.list_skills()
    }

    /// Get the state of a specific skill.
    pub fn get_skill_state(&self, skill_id: &str) -> Option<SkillSnapshot> {
        self.registry.get_skill(skill_id)
    }

    /// Call a tool on a skill.
    pub async fn call_tool(
        &self,
        skill_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolResult, String> {
        self.registry
            .call_tool(skill_id, tool_name, arguments)
            .await
    }

    /// Broadcast a server event to all running skills.
    pub async fn broadcast_event(&self, event: &str, data: serde_json::Value) {
        self.registry.broadcast_event(event, data).await;
    }

    /// Get all tool definitions across all running skills.
    pub fn all_tools(&self) -> Vec<(String, crate::openhuman::skills::types::ToolDefinition)> {
        self.registry.all_tools()
    }

    /// Emit a skill status change event to the frontend.
    fn emit_status_change(&self, skill_id: &str) {
        if let Some(ref app) = *self.app_handle.read() {
            if let Some(snap) = self.registry.get_skill(skill_id) {
                let _ = app.emit(events::SKILL_STATUS_CHANGED, &snap);
            }
        }
    }

    /// Auto-start skills based on user preferences.
    /// No stagger delay needed for QuickJS (lightweight contexts).
    pub async fn auto_start_skills(&self) {
        match self.discover_skills().await {
            Ok(manifests) => {
                for manifest in manifests {
                    let should_start = self
                        .preferences
                        .resolve_should_start(&manifest.id, manifest.auto_start);
                    if should_start {
                        log::info!(
                            "[runtime] Auto-starting skill: {} ({})",
                            manifest.name,
                            manifest.id
                        );
                        if let Err(e) = self.start_skill(&manifest.id).await {
                            log::error!(
                                "[runtime] Failed to auto-start skill '{}': {e}",
                                manifest.id
                            );
                        }
                    }
                }
            }
            Err(e) => {
                log::warn!("[runtime] Failed to discover skills for auto-start: {e}");
            }
        }
        // Sync all tool state to backend after auto-start completes
        self.sync_tools().await;
    }

    /// Enable a skill.
    pub async fn enable_skill(&self, skill_id: &str) -> Result<(), String> {
        self.preferences.set_enabled(skill_id, true);
        self.start_skill(skill_id).await?;
        Ok(())
    }

    /// Disable a skill.
    pub async fn disable_skill(&self, skill_id: &str) -> Result<(), String> {
        self.preferences.set_enabled(skill_id, false);
        if self.registry.has_skill(skill_id) {
            self.stop_skill(skill_id).await?;
        }
        Ok(())
    }

    /// Check whether a skill is enabled.
    pub fn is_skill_enabled(&self, skill_id: &str) -> bool {
        self.preferences.is_enabled(skill_id).unwrap_or(false)
    }

    /// Get all stored preferences.
    pub fn get_preferences(
        &self,
    ) -> std::collections::HashMap<String, crate::openhuman::skills::preferences::SkillPreference>
    {
        self.preferences.get_all()
    }

    /// Read a KV value from a skill's database.
    /// TODO: Removed during runtime cleanup - reimplement if needed
    pub fn kv_get(&self, _skill_id: &str, _key: &str) -> Result<serde_json::Value, String> {
        Err("KV storage removed during runtime cleanup".to_string())
    }

    /// Write a KV value into a skill's database.
    /// TODO: Removed during runtime cleanup - reimplement if needed
    pub fn kv_set(
        &self,
        _skill_id: &str,
        _key: &str,
        _value: &serde_json::Value,
    ) -> Result<(), String> {
        Err("KV storage removed during runtime cleanup".to_string())
    }

    /// Route a JSON-RPC method call.
    pub async fn rpc(
        &self,
        skill_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        use crate::openhuman::skills::types::SkillMessage;

        match method {
            "skill/load" => {
                // Extract load params (exclude manifest and dataDir) and send to skill
                let load_params: serde_json::Map<String, serde_json::Value> = params
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .filter(|(k, _)| k.as_str() != "manifest" && k.as_str() != "dataDir")
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                if !load_params.is_empty() {
                    let msg = SkillMessage::LoadParams {
                        params: serde_json::Value::Object(load_params),
                    };
                    if let Err(e) = self.registry.send_message(skill_id, msg) {
                        log::warn!(
                            "[runtime] Failed to send LoadParams to skill '{}': {}",
                            skill_id,
                            e
                        );
                    }
                }
                Ok(serde_json::json!({ "ok": true }))
            }

            "setup/start" => {
                log::info!("[runtime] setup/start for '{}'", skill_id);
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.registry
                    .send_message(skill_id, SkillMessage::SetupStart { reply: tx })?;
                rx.await
                    .map_err(|_| "SetupStart channel closed".to_string())?
            }

            "setup/submit" => {
                let step_id = params
                    .get("stepId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let values = params
                    .get("values")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.registry.send_message(
                    skill_id,
                    SkillMessage::SetupSubmit {
                        step_id,
                        values,
                        reply: tx,
                    },
                )?;
                rx.await
                    .map_err(|_| "SetupSubmit channel closed".to_string())?
            }

            "setup/cancel" => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.registry
                    .send_message(skill_id, SkillMessage::SetupCancel { reply: tx })?;
                rx.await
                    .map_err(|_| "SetupCancel channel closed".to_string())?
                    .map(|()| serde_json::json!({ "ok": true }))
            }

            "tools/list" => {
                let snap = self.registry.get_skill(skill_id);
                let tools = snap
                    .map(|s| {
                        s.tools
                            .iter()
                            .map(|t| serde_json::to_value(t).unwrap_or_default())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                Ok(serde_json::json!({ "tools": tools }))
            }

            "tools/call" => {
                let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));
                let result = self.call_tool(skill_id, tool_name, arguments).await?;
                serde_json::to_value(&result)
                    .map_err(|e| format!("Failed to serialize tool result: {e}"))
            }

            "options/list" => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.registry
                    .send_message(skill_id, SkillMessage::ListOptions { reply: tx })?;
                rx.await
                    .map_err(|_| "ListOptions channel closed".to_string())?
            }

            "options/set" => {
                let name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let value = params
                    .get("value")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.registry.send_message(
                    skill_id,
                    SkillMessage::SetOption {
                        name,
                        value,
                        reply: tx,
                    },
                )?;
                rx.await
                    .map_err(|_| "SetOption channel closed".to_string())?
                    .map(|()| serde_json::json!({ "ok": true }))
            }

            "skill/tick" => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.registry
                    .send_message(skill_id, SkillMessage::Tick { reply: tx })?;
                rx.await
                    .map_err(|_| "Tick channel closed".to_string())?
                    .map(|()| serde_json::json!({ "ok": true }))
            }

            "skill/sessionStart" => {
                let session_id = params
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.registry.send_message(
                    skill_id,
                    SkillMessage::SessionStart {
                        session_id,
                        reply: tx,
                    },
                )?;
                rx.await
                    .map_err(|_| "SessionStart channel closed".to_string())?
                    .map(|()| serde_json::json!({ "ok": true }))
            }

            "skill/sessionEnd" => {
                let session_id = params
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.registry.send_message(
                    skill_id,
                    SkillMessage::SessionEnd {
                        session_id,
                        reply: tx,
                    },
                )?;
                rx.await
                    .map_err(|_| "SessionEnd channel closed".to_string())?
                    .map(|()| serde_json::json!({ "ok": true }))
            }

            "skill/shutdown" => {
                self.stop_skill(skill_id).await?;
                Ok(serde_json::json!({ "ok": true }))
            }

            _ => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.registry.send_message(
                    skill_id,
                    SkillMessage::Rpc {
                        method: method.to_string(),
                        params,
                        reply: tx,
                    },
                )?;
                rx.await.map_err(|_| "Rpc channel closed".to_string())?
            }
        }
    }

    /// Read a file from a skill's data directory.
    pub fn data_read(&self, skill_id: &str, filename: &str) -> Result<String, String> {
        let data_dir = self.skills_data_dir.join(skill_id);
        let path = data_dir.join(filename);
        std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read data file '{}': {e}", filename))
    }

    /// Write a file to a skill's data directory.
    pub fn data_write(&self, skill_id: &str, filename: &str, content: &str) -> Result<(), String> {
        let data_dir = self.skills_data_dir.join(skill_id);
        let path = data_dir.join(filename);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create data dir: {e}"))?;
        }
        std::fs::write(&path, content)
            .map_err(|e| format!("Failed to write data file '{}': {e}", filename))
    }

    /// Get the data directory path for a skill.
    pub fn skill_data_dir(&self, skill_id: &str) -> PathBuf {
        self.skills_data_dir.join(skill_id)
    }
}
