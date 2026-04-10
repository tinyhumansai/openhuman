//! Implementation of the QuickJS skill instance.
//!
//! This module contains the main logic for initializing and spawning the execution
//! loop of a QuickJS-based skill. It handles setting up the runtime, context,
//! registering native bridges, loading the skill bundle, and transitioning through
//! lifecycle stages (init, start, running).

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::mpsc;

use crate::openhuman::skills::quickjs_libs::{qjs_ops, IdbStorage};
use crate::openhuman::skills::types::{SkillConfig, SkillMessage, SkillSnapshot, SkillStatus};

use super::event_loop::run_event_loop;
use super::js_handlers::{call_lifecycle, handle_js_call};
use super::js_helpers::{
    drive_jobs, extract_tools, format_js_exception, restore_auth_credential, restore_client_key,
    restore_oauth_credential,
};
use super::types::{BridgeDeps, QjsSkillInstance, SkillState};

/// Read persisted oauth/auth credentials from a skill's data directory and
/// produce a JS expression suitable for `start({ oauth, auth })`.
///
/// This is the canonical shape passed to `start()`: a single object with
/// `oauth` and `auth` keys, each either an object or `null`. Skills can read
/// either field directly or rely on the runtime bridges that have already
/// been populated by the `restore_*_credential` helpers.
pub(crate) fn build_start_credentials_arg(data_dir: &std::path::Path) -> String {
    fn read_json(path: &std::path::Path) -> serde_json::Value {
        match std::fs::read_to_string(path) {
            Ok(s) if !s.trim().is_empty() => {
                serde_json::from_str(&s).unwrap_or(serde_json::Value::Null)
            }
            _ => serde_json::Value::Null,
        }
    }

    let oauth = read_json(&data_dir.join("oauth_credential.json"));
    let auth = read_json(&data_dir.join("auth_credential.json"));
    let arg = serde_json::json!({
        "oauth": oauth,
        "auth": auth,
    });
    serde_json::to_string(&arg).unwrap_or_else(|_| "{\"oauth\":null,\"auth\":null}".to_string())
}

impl QjsSkillInstance {
    /// Create a new QuickJS skill instance.
    ///
    /// Returns the instance and a channel to receive messages from the system.
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
    ///
    /// Note: `setup_complete` and `connection_status` are populated later
    /// by `RuntimeEngine::enrich_snapshot()` which has access to `PreferencesStore`.
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
    ///
    /// This function sets up the entire QuickJS environment, including memory limits,
    /// native bridge registration (ops), bootstrap code, and the skill's own bundle.
    /// It then transitions through the `init()` and `start()` lifecycles before
    /// entering the main event loop.
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
            // Update status to Initializing as we begin setup
            state.write().status = SkillStatus::Initializing;

            // Create persistent storage (IndexedDB-like) in the skill's data directory
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

            // Read the entry point JS file from the skill bundle directory
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

            // Create a fresh QuickJS runtime. QuickJS allows multiple runtimes in one process.
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

            // Apply resource constraints to the runtime
            rt.set_memory_limit(config.memory_limit).await;
            rt.set_max_stack_size(512 * 1024).await; // 512KB stack is usually plenty for skills

            // Create context with the full standard library (Date, Math, JSON, etc.)
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

            // Prepare internal state containers for JS features
            let timer_state = Arc::new(RwLock::new(qjs_ops::TimerState::default()));
            let ws_state = Arc::new(RwLock::new(qjs_ops::WebSocketState::default()));
            let published_state = Arc::new(RwLock::new(qjs_ops::SkillState::default()));

            // Register native bridges (ops) and perform bootstrap
            let skill_id = config.skill_id.clone();
            let init_result = ctx
                .with(|js_ctx| {
                    // SkillContext contains dependencies required by native bridges
                    let skill_context = qjs_ops::SkillContext {
                        skill_id: skill_id.clone(),
                        data_dir: data_dir.clone(),
                        memory_client: deps.memory_client.clone(),
                        webhook_router: deps.webhook_router.clone(),
                    };

                    // Register functions on the global scope via the __ops bridge
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

                    // Load bootstrap code to set up high-level JS APIs (Fetch, Timers, etc.)
                    let bootstrap_code = include_str!("../quickjs_libs/bootstrap.js");
                    if let Err(e) = js_ctx.eval::<rquickjs::Value, _>(bootstrap_code) {
                        let detail = format_js_exception(&js_ctx, &e);
                        return Err(format!("Bootstrap failed: {detail}"));
                    }

                    // Inject the skill ID into the global scope for internal JS use
                    let bridge_code = format!(
                        r#"globalThis.__skillId = "{}";"#,
                        skill_id.replace('"', r#"\""#)
                    );
                    if let Err(e) = js_ctx.eval::<rquickjs::Value, _>(bridge_code.as_bytes()) {
                        let detail = format_js_exception(&js_ctx, &e);
                        return Err(format!("Skill init failed: {detail}"));
                    }

                    // Evaluate the actual skill bundle code
                    if let Err(e) = js_ctx.eval::<rquickjs::Value, _>(js_source.as_bytes()) {
                        let detail = format_js_exception(&js_ctx, &e);
                        return Err(format!("Skill load failed: {detail}"));
                    }

                    // Inspect the global scope to extract tool definitions (tools_list)
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

            // Restore previously saved authentication state from the data directory
            restore_oauth_credential(&ctx, &config.skill_id, &data_dir).await;
            restore_auth_credential(&ctx, &config.skill_id, &data_dir).await;
            restore_client_key(&ctx, &config.skill_id, &data_dir).await;

            // Trigger the `init()` lifecycle callback in the JS skill
            if let Err(e) = call_lifecycle(&rt, &ctx, "init", None).await {
                let mut s = state.write();
                s.status = SkillStatus::Error;
                s.error = Some(format!("init() failed: {e}"));
                log::error!("[skill:{}] init() failed: {e}", config.skill_id);
                return;
            }

            // Execute any microtasks or pending promises scheduled during init()
            drive_jobs(&rt).await;

            // Build the credential bag passed to `start(creds)`. We read the
            // persisted credential files from disk so the skill receives the
            // canonical view that matches what the bridges (`oauth`, `auth`)
            // already see in JS land. If no creds are stored we still pass an
            // explicit `{ oauth: null, auth: null }` so start() always has a
            // well-defined shape — that is the whole point of this contract.
            let start_args = build_start_credentials_arg(&data_dir);
            log::info!(
                "[skill:{}] Calling start() with credentials (oauth={}, auth={})",
                config.skill_id,
                !start_args.contains("\"oauth\":null"),
                !start_args.contains("\"auth\":null"),
            );

            // Trigger the `start()` lifecycle callback
            if let Err(e) = call_lifecycle(&rt, &ctx, "start", Some(&start_args)).await {
                let mut s = state.write();
                s.status = SkillStatus::Error;
                s.error = Some(format!("start() failed: {e}"));
                log::error!("[skill:{}] start() failed: {e}", config.skill_id);
                return;
            }

            // Execute any microtasks or pending promises scheduled during start()
            drive_jobs(&rt).await;

            // Mark the skill as officially running
            state.write().status = SkillStatus::Running;
            log::info!("[skill:{}] Running (QuickJS)", config.skill_id);

            // Execute an initial `onPing` call to verify the JS -> Bridge connection is healthy
            match handle_js_call(&rt, &ctx, "onPing", "{}").await {
                Ok(value) => {
                    log::info!("[skill:{}] Initial ping result: {}", config.skill_id, value);
                }
                Err(e) => {
                    log::warn!("[skill:{}] Initial ping failed: {}", config.skill_id, e);
                }
            }
            drive_jobs(&rt).await;

            // Hand over control to the main event loop which waits for system/RPC messages
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
