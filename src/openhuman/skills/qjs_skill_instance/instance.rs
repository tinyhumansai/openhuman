use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::mpsc;

use crate::openhuman::skills::quickjs_libs::{qjs_ops, IdbStorage};
use crate::openhuman::skills::types::{SkillConfig, SkillMessage, SkillSnapshot, SkillStatus};

use super::event_loop::run_event_loop;
use super::js_handlers::{call_lifecycle, handle_js_call};
use super::js_helpers::{
    drive_jobs, extract_tools, format_js_exception, restore_auth_credential,
    restore_client_key, restore_oauth_credential,
};
use super::types::{BridgeDeps, QjsSkillInstance, SkillState};

impl QjsSkillInstance {
    /// Create a new QuickJS skill instance.
    pub fn new(
        config: SkillConfig,
        skill_dir: PathBuf,
        data_dir: PathBuf,
    ) -> (Self, mpsc::Receiver<SkillMessage>) {
        let (tx, rx) = mpsc::channel(64);
        let instance = Self {
            config,
            state: Arc::new(RwLock::new(SkillState::default())),
            sender: tx,
            skill_dir,
            data_dir,
        };
        (instance, rx)
    }

    /// Take a snapshot of the current skill state.
    /// Note: `setup_complete` and `connection_status` are populated later
    /// by RuntimeEngine::enrich_snapshot() which has access to PreferencesStore.
    pub fn snapshot(&self) -> SkillSnapshot {
        let state = self.state.read();
        SkillSnapshot {
            skill_id: self.config.skill_id.clone(),
            name: self.config.name.clone(),
            status: state.status,
            tools: state.tools.clone(),
            error: state.error.clone(),
            state: state.published_state.clone(),
            setup_complete: false,
            connection_status: String::new(),
        }
    }

    /// Spawn the skill's execution loop as a tokio task.
    /// Unlike V8 (which needed spawn_blocking), QuickJS contexts are Send.
    pub fn spawn(
        &self,
        mut rx: mpsc::Receiver<SkillMessage>,
        deps: BridgeDeps,
    ) -> tokio::task::JoinHandle<()> {
        let config = self.config.clone();
        let state = self.state.clone();
        let skill_dir = self.skill_dir.clone();
        let data_dir = self.data_dir.clone();

        tokio::spawn(async move {
            // Update status
            state.write().status = SkillStatus::Initializing;

            // Create storage
            let storage: IdbStorage = match IdbStorage::new(&data_dir) {
                Ok(s) => s,
                Err(e) => {
                    let mut s = state.write();
                    s.status = SkillStatus::Error;
                    s.error = Some(format!("Failed to create storage: {e}"));
                    log::error!("[skill:{}] Storage creation failed: {e}", config.skill_id);
                    return;
                }
            };

            // Read the entry point JS file
            let entry_path = skill_dir.join(&config.entry_point);
            let js_source = match tokio::fs::read_to_string(&entry_path).await {
                Ok(src) => src,
                Err(e) => {
                    let mut s = state.write();
                    s.status = SkillStatus::Error;
                    s.error = Some(format!("Failed to read {}: {e}", config.entry_point));
                    log::error!(
                        "[skill:{}] Failed to read entry point: {e}",
                        config.skill_id
                    );
                    return;
                }
            };

            // Create QuickJS runtime with memory limits
            let rt = match rquickjs::AsyncRuntime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    let mut s = state.write();
                    s.status = SkillStatus::Error;
                    s.error = Some(format!("Failed to create QuickJS runtime: {e}"));
                    log::error!(
                        "[skill:{}] Failed to create QuickJS runtime: {e}",
                        config.skill_id
                    );
                    return;
                }
            };

            // Set memory limit (config.memory_limit is in bytes)
            rt.set_memory_limit(config.memory_limit).await;
            rt.set_max_stack_size(512 * 1024).await; // 512KB stack

            // Create context with full standard library
            let ctx = match rquickjs::AsyncContext::full(&rt).await {
                Ok(ctx) => ctx,
                Err(e) => {
                    let mut s = state.write();
                    s.status = SkillStatus::Error;
                    s.error = Some(format!("Failed to create QuickJS context: {e}"));
                    log::error!("[skill:{}] Failed to create context: {e}", config.skill_id);
                    return;
                }
            };

            // Create shared timer state
            let timer_state = Arc::new(RwLock::new(qjs_ops::TimerState::default()));

            // Create WebSocket state
            let ws_state = Arc::new(RwLock::new(qjs_ops::WebSocketState::default()));

            // Create published skill state (different from SkillState above)
            let published_state = Arc::new(RwLock::new(qjs_ops::SkillState::default()));

            // Register ops and run bootstrap + skill code
            let skill_id = config.skill_id.clone();
            let init_result = ctx
                .with(|js_ctx| {
                    // Register native ops as __ops global
                    let skill_context = qjs_ops::SkillContext {
                        skill_id: skill_id.clone(),
                        data_dir: data_dir.clone(),
                        memory_client: deps.memory_client.clone(),
                        webhook_router: deps.webhook_router.clone(),
                    };

                    if let Err(e) = qjs_ops::register_ops(
                        &js_ctx,
                        storage.clone(),
                        skill_context,
                        published_state.clone(),
                        timer_state.clone(),
                        ws_state.clone(),
                    ) {
                        return Err(format!("Failed to register ops: {e}"));
                    }

                    // Load bootstrap
                    let bootstrap_code = include_str!("../quickjs_libs/bootstrap.js");
                    if let Err(e) = js_ctx.eval::<rquickjs::Value, _>(bootstrap_code) {
                        let detail = format_js_exception(&js_ctx, &e);
                        return Err(format!("Bootstrap failed: {detail}"));
                    }

                    // Set skill ID
                    let bridge_code = format!(
                        r#"globalThis.__skillId = "{}";"#,
                        skill_id.replace('"', r#"\""#)
                    );
                    if let Err(e) = js_ctx.eval::<rquickjs::Value, _>(bridge_code.as_bytes()) {
                        let detail = format_js_exception(&js_ctx, &e);
                        return Err(format!("Skill init failed: {detail}"));
                    }

                    // Execute the skill's entry point
                    if let Err(e) = js_ctx.eval::<rquickjs::Value, _>(js_source.as_bytes()) {
                        let detail = format_js_exception(&js_ctx, &e);
                        return Err(format!("Skill load failed: {detail}"));
                    }

                    // Extract tool definitions
                    extract_tools(&js_ctx, &state);

                    Ok(())
                })
                .await;

            if let Err(e) = init_result {
                let mut s = state.write();
                s.status = SkillStatus::Error;
                s.error = Some(e.clone());
                log::error!("[skill:{}] {e}", config.skill_id);
                return;
            }

            restore_oauth_credential(&ctx, &config.skill_id, &data_dir).await;
            restore_auth_credential(&ctx, &config.skill_id, &data_dir).await;
            restore_client_key(&ctx, &config.skill_id, &data_dir).await;

            // Call init() lifecycle
            if let Err(e) = call_lifecycle(&rt, &ctx, "init").await {
                let mut s = state.write();
                s.status = SkillStatus::Error;
                s.error = Some(format!("init() failed: {e}"));
                log::error!("[skill:{}] init() failed: {e}", config.skill_id);
                return;
            }

            // Execute pending jobs after init (process promises)
            drive_jobs(&rt).await;

            // Call start() lifecycle
            if let Err(e) = call_lifecycle(&rt, &ctx, "start").await {
                let mut s = state.write();
                s.status = SkillStatus::Error;
                s.error = Some(format!("start() failed: {e}"));
                log::error!("[skill:{}] start() failed: {e}", config.skill_id);
                return;
            }

            // Execute pending jobs after start
            drive_jobs(&rt).await;

            // Mark as running
            state.write().status = SkillStatus::Running;
            log::info!("[skill:{}] Running (QuickJS)", config.skill_id);

            // Immediate ping to verify the connection is healthy
            match handle_js_call(&rt, &ctx, "onPing", "{}").await {
                Ok(value) => {
                    log::info!("[skill:{}] Initial ping result: {}", config.skill_id, value);
                }
                Err(e) => {
                    log::warn!("[skill:{}] Initial ping failed: {}", config.skill_id, e);
                }
            }
            drive_jobs(&rt).await;

            // Run the event loop
            run_event_loop(
                &rt,
                &ctx,
                &mut rx,
                &state,
                &config.skill_id,
                &timer_state,
                &published_state,
                deps.memory_client.clone(),
                &data_dir,
            )
            .await;
        })
    }
}
