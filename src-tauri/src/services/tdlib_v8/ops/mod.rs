//! Custom Deno Core Ops for V8 Runtime
//!
//! Provides browser API implementations as Rust ops.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use deno_core::{extension, op2, OpState};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::storage::IdbStorage;

// ============================================================================
// Extension Definition
// ============================================================================

extension!(
    alphahuman_ops,
    ops = [
        // Console ops
        op_console_log,
        op_console_warn,
        op_console_error,
        // Crypto ops
        op_crypto_random,
        op_atob,
        op_btoa,
        // Performance ops
        op_performance_now,
        // Platform ops
        op_platform_os,
        op_platform_env,
        // Timer ops (prefixed to avoid conflict with deno_core built-ins)
        op_ah_timer_start,
        op_ah_timer_cancel,
        // Fetch ops
        op_fetch,
        // WebSocket ops
        op_ws_connect,
        op_ws_send,
        op_ws_recv,
        op_ws_close,
        // IndexedDB ops
        op_idb_open,
        op_idb_close,
        op_idb_delete_database,
        op_idb_create_object_store,
        op_idb_delete_object_store,
        op_idb_get,
        op_idb_put,
        op_idb_delete,
        op_idb_clear,
        op_idb_get_all,
        op_idb_get_all_keys,
        op_idb_count,
        // Skill bridge ops
        op_db_exec,
        op_db_get,
        op_db_all,
        op_db_kv_get,
        op_db_kv_set,
        op_store_get,
        op_store_set,
        op_store_delete,
        op_store_keys,
        op_net_fetch,
        // State bridge ops
        op_state_get,
        op_state_set,
        op_state_set_partial,
        // Data bridge ops
        op_data_read,
        op_data_write,
        // TDLib ops (telegram skill only)
        op_tdlib_create_client,
        op_tdlib_send,
        op_tdlib_receive,
        op_tdlib_destroy,
        op_tdlib_is_available,
        // Model ops (local LLM inference)
        op_model_is_available,
        op_model_get_status,
        op_model_generate,
        op_model_summarize,
    ],
    state = |state| {
        // State will be initialized when runtime is created
    }
);

/// Build the deno_core Extension with all custom ops.
pub fn build_extension(_storage: IdbStorage) -> deno_core::Extension {
    alphahuman_ops::init_ops_and_esm()
}

/// Initialize storage in op state with data directory and shared skill state.
pub fn init_state_with_data_dir(
    state: &mut OpState,
    storage: IdbStorage,
    skill_id: String,
    data_dir: std::path::PathBuf,
    skill_state: std::sync::Arc<parking_lot::RwLock<crate::runtime::v8_skill_instance::SkillState>>,
) {
    state.put(storage);
    state.put(SkillContext::with_state(skill_id, data_dir, skill_state));
    state.put(TimerState::default());
    state.put(WebSocketState::default());
}

/// Poll timers and return IDs of timers that are ready to fire.
/// Also returns the duration until the next timer (for efficient sleeping).
pub fn poll_timers(state: &mut OpState) -> (Vec<u32>, Option<std::time::Duration>) {
    let timer_state = state.borrow_mut::<TimerState>();
    let ready = timer_state.poll_ready();
    let next = timer_state.time_until_next();
    (ready, next)
}

/// Context for the current skill execution.
#[derive(Clone)]
pub struct SkillContext {
    pub skill_id: String,
    /// Skill-specific data directory for file I/O.
    pub data_dir: Option<std::path::PathBuf>,
    /// Shared reference to the skill's state (same instance as V8SkillInstance).
    pub skill_state: Option<std::sync::Arc<parking_lot::RwLock<crate::runtime::v8_skill_instance::SkillState>>>,
}

impl Default for SkillContext {
    fn default() -> Self {
        Self {
            skill_id: String::new(),
            data_dir: None,
            skill_state: None,
        }
    }
}

impl SkillContext {
    /// Create a new SkillContext with a skill ID, data directory, and shared state.
    pub fn with_state(
        skill_id: String,
        data_dir: std::path::PathBuf,
        skill_state: std::sync::Arc<parking_lot::RwLock<crate::runtime::v8_skill_instance::SkillState>>,
    ) -> Self {
        Self {
            skill_id,
            data_dir: Some(data_dir),
            skill_state: Some(skill_state),
        }
    }
}

// ============================================================================
// Timer State
// ============================================================================

/// A scheduled timer (setTimeout or setInterval).
#[derive(Clone, Debug)]
pub struct TimerEntry {
    /// Whether this is a repeating interval
    pub is_interval: bool,
    /// Delay in milliseconds
    pub delay_ms: u64,
    /// When the timer should fire next (Instant)
    pub next_fire: std::time::Instant,
}

/// State for managing timers.
/// Timers are polled during the event loop and fire callbacks via JS.
#[derive(Default)]
pub struct TimerState {
    /// Active timers: id -> TimerEntry
    pub timers: HashMap<u32, TimerEntry>,
}

impl TimerState {
    /// Get all timers that are ready to fire, returning their IDs.
    /// For intervals, reschedules the next fire time.
    /// For timeouts, removes them from the map.
    pub fn poll_ready(&mut self) -> Vec<u32> {
        let now = std::time::Instant::now();
        let mut ready = Vec::new();

        // Collect IDs of ready timers
        let ready_ids: Vec<u32> = self
            .timers
            .iter()
            .filter(|(_, entry)| now >= entry.next_fire)
            .map(|(id, _)| *id)
            .collect();

        for id in ready_ids {
            if let Some(entry) = self.timers.get_mut(&id) {
                ready.push(id);

                if entry.is_interval {
                    // Reschedule for next interval
                    entry.next_fire = now + std::time::Duration::from_millis(entry.delay_ms);
                } else {
                    // Remove one-shot timeout
                    self.timers.remove(&id);
                }
            }
        }

        ready
    }

    /// Get the duration until the next timer fires (for sleep optimization).
    pub fn time_until_next(&self) -> Option<std::time::Duration> {
        let now = std::time::Instant::now();
        self.timers
            .values()
            .map(|entry| {
                if entry.next_fire > now {
                    entry.next_fire - now
                } else {
                    std::time::Duration::ZERO
                }
            })
            .min()
    }
}

// ============================================================================
// WebSocket State
// ============================================================================

/// State for managing WebSocket connections.
/// Currently a placeholder - full state management is TODO.
#[derive(Default)]
#[allow(dead_code)]
pub struct WebSocketState {
    next_id: u32,
    /// Active connections: id -> WebSocket sender
    pub connections: HashMap<u32, WebSocketConnection>,
}

#[allow(dead_code)]
pub struct WebSocketConnection {
    pub write_tx: mpsc::Sender<String>,
    pub read_rx: mpsc::Receiver<String>,
    pub close_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

// ============================================================================
// Console Ops
// ============================================================================

#[op2(fast)]
fn op_console_log(#[string] msg: &str) {
    log::info!("[js] {}", msg);
}

#[op2(fast)]
fn op_console_warn(#[string] msg: &str) {
    log::warn!("[js] {}", msg);
}

#[op2(fast)]
fn op_console_error(#[string] msg: &str) {
    log::error!("[js] {}", msg);
}

// ============================================================================
// Crypto Ops
// ============================================================================

#[op2]
#[buffer]
fn op_crypto_random(len: u32) -> Vec<u8> {
    use rand::RngCore;
    let mut bytes = vec![0u8; len as usize];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

#[op2]
#[string]
fn op_atob(#[string] input: &str) -> Result<String, deno_core::error::AnyError> {
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD.decode(input)?;
    Ok(String::from_utf8_lossy(&decoded).to_string())
}

#[op2]
#[string]
fn op_btoa(#[string] input: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(input.as_bytes())
}

// ============================================================================
// Performance Ops
// ============================================================================

#[op2(fast)]
fn op_performance_now() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

// ============================================================================
// Platform Ops
// ============================================================================

#[op2]
#[string]
fn op_platform_os() -> &'static str {
    #[cfg(target_os = "windows")]
    return "windows";
    #[cfg(target_os = "macos")]
    return "macos";
    #[cfg(target_os = "linux")]
    return "linux";
    #[cfg(target_os = "android")]
    return "android";
    #[cfg(target_os = "ios")]
    return "ios";
    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "linux",
        target_os = "android",
        target_os = "ios"
    )))]
    return "unknown";
}

#[op2]
#[string]
fn op_platform_env(#[string] key: &str) -> Option<String> {
    const ALLOWED_ENV_VARS: &[&str] = &[
        "VITE_TELEGRAM_BOT_USERNAME",
        "VITE_TELEGRAM_BOT_ID",
        "TELEGRAM_API_ID",   // Skills may use this without VITE_ prefix
        "TELEGRAM_API_HASH", // Skills may use this without VITE_ prefix
        "VITE_BACKEND_URL",
        "BACKEND_URL", // Skills may use this without VITE_ prefix
        "VITE_DEBUG",
    ];

    if ALLOWED_ENV_VARS.contains(&key) {
        std::env::var(key).ok()
    } else {
        None
    }
}

// ============================================================================
// Timer Ops
// ============================================================================

/// Start a timer (setTimeout or setInterval).
/// The actual callback execution happens in JavaScript via __handleTimer,
/// triggered by the event loop polling TimerState.
#[op2(fast)]
fn op_ah_timer_start(state: &mut OpState, id: u32, delay_ms: u32, is_interval: bool) {
    let timer_state = state.borrow_mut::<TimerState>();

    let entry = TimerEntry {
        is_interval,
        delay_ms: delay_ms as u64,
        next_fire: std::time::Instant::now() + std::time::Duration::from_millis(delay_ms as u64),
    };

    timer_state.timers.insert(id, entry);
    log::debug!(
        "[timer] Registered {} {} with delay {}ms",
        if is_interval { "interval" } else { "timeout" },
        id,
        delay_ms
    );
}

/// Cancel a timer.
#[op2(fast)]
fn op_ah_timer_cancel(state: &mut OpState, id: u32) {
    let timer_state = state.borrow_mut::<TimerState>();

    if timer_state.timers.remove(&id).is_some() {
        log::debug!("[timer] Cancelled timer {}", id);
    }
}

// ============================================================================
// Fetch Ops
// ============================================================================

#[derive(Deserialize)]
struct FetchOptions {
    method: Option<String>,
    headers: Option<HashMap<String, String>>,
    body: Option<String>,
}

#[derive(Serialize)]
struct FetchResponse {
    status: u16,
    #[serde(rename = "statusText")]
    status_text: String,
    headers: HashMap<String, String>,
    body: String,
}

/// Async fetch operation for the fetch API.
#[op2(async)]
#[serde]
async fn op_fetch(
    #[string] url: String,
    #[serde] options: FetchOptions,
) -> Result<FetchResponse, deno_core::error::AnyError> {
    let client = reqwest::Client::new();

    let method = options.method.unwrap_or_else(|| "GET".to_string());
    let method: reqwest::Method = method.parse().map_err(|_| {
        deno_core::error::generic_error(format!("Invalid HTTP method: {}", method))
    })?;

    let mut request = client.request(method, &url);

    // Add headers
    if let Some(headers) = options.headers {
        for (key, value) in headers {
            request = request.header(&key, &value);
        }
    }

    // Add body
    if let Some(body) = options.body {
        request = request.body(body);
    }

    let response = request.send().await.map_err(|e| {
        deno_core::error::generic_error(format!("Fetch failed: {}", e))
    })?;

    let status = response.status().as_u16();
    let status_text = response.status().canonical_reason().unwrap_or("").to_string();

    let mut headers = HashMap::new();
    for (key, value) in response.headers() {
        if let Ok(v) = value.to_str() {
            headers.insert(key.to_string(), v.to_string());
        }
    }

    let body = response.text().await.map_err(|e| {
        deno_core::error::generic_error(format!("Failed to read response body: {}", e))
    })?;

    Ok(FetchResponse {
        status,
        status_text,
        headers,
        body,
    })
}

// ============================================================================
// WebSocket Ops
// ============================================================================

/// Connect to a WebSocket server.
#[op2(async)]
async fn op_ws_connect(
    #[string] url: String,
) -> Result<u32, deno_core::error::AnyError> {
    use futures::StreamExt;
    use tokio_tungstenite::connect_async;

    let (ws_stream, _) = connect_async(&url).await.map_err(|e| {
        deno_core::error::generic_error(format!("WebSocket connect failed: {}", e))
    })?;

    let (write, mut read) = ws_stream.split();

    // Create channels for communication
    // Note: These are set up for future full WebSocket state management
    let (_write_tx, write_rx) = mpsc::channel::<String>(32);
    let (read_tx, _read_rx) = mpsc::channel::<String>(32);
    let (_close_tx, close_rx) = tokio::sync::oneshot::channel::<()>();

    // Spawn write task
    let _write_handle = tokio::spawn(async move {
        use futures::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        let mut write = write;
        let mut write_rx_inner = write_rx;
        let mut close_rx = close_rx;

        loop {
            tokio::select! {
                msg = write_rx_inner.recv() => {
                    match msg {
                        Some(text) => {
                            if write.send(Message::Text(text)).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                _ = &mut close_rx => {
                    let _ = write.close().await;
                    break;
                }
            }
        }
    });

    // Spawn read task
    let _read_handle = tokio::spawn(async move {
        use tokio_tungstenite::tungstenite::Message;

        while let Some(result) = read.next().await {
            match result {
                Ok(Message::Text(text)) => {
                    if read_tx.send(text).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Binary(data)) => {
                    // Convert binary to base64 for JavaScript
                    use base64::Engine;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                    if read_tx.send(b64).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(_) => break,
                _ => {} // Ignore ping/pong
            }
        }
    });

    // Generate a simple ID (in production, use proper state management)
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u32)
        .unwrap_or(1);

    log::info!("[ws] Connected to {} with id {}", url, id);

    Ok(id)
}

/// Send a message over WebSocket.
#[op2(fast)]
fn op_ws_send(
    _state: &mut OpState,
    _id: u32,
    #[string] _data: &str,
) -> Result<(), deno_core::error::AnyError> {
    // Note: Full WebSocket state management would require more complex handling
    // For now, this is a placeholder
    log::debug!("[ws] Send message on connection {}", _id);
    Ok(())
}

/// Receive a message from WebSocket (async).
#[op2(async)]
#[string]
async fn op_ws_recv(_id: u32) -> Result<Option<String>, deno_core::error::AnyError> {
    // Note: Full WebSocket state management would require more complex handling
    // For now, return None to indicate no message
    Ok(None)
}

/// Close a WebSocket connection.
#[op2(fast)]
fn op_ws_close(
    _state: &mut OpState,
    _id: u32,
    _code: u16,
    #[string] _reason: &str,
) -> Result<(), deno_core::error::AnyError> {
    log::debug!("[ws] Close connection {}", _id);
    Ok(())
}

// ============================================================================
// IndexedDB Ops
// ============================================================================

#[derive(Serialize)]
struct IdbOpenResult {
    #[serde(rename = "needsUpgrade")]
    needs_upgrade: bool,
    #[serde(rename = "oldVersion")]
    old_version: u32,
    #[serde(rename = "objectStores")]
    object_stores: Vec<String>,
}

/// Open an IndexedDB database.
#[op2(async)]
#[serde]
async fn op_idb_open(
    state: Rc<RefCell<OpState>>,
    #[string] name: String,
    version: u32,
) -> Result<IdbOpenResult, deno_core::error::AnyError> {
    let storage = {
        let state = state.borrow();
        state.borrow::<IdbStorage>().clone()
    };

    let result = storage.open_database(&name, version).await.map_err(|e| {
        deno_core::error::generic_error(format!("IDB open failed: {}", e))
    })?;

    Ok(IdbOpenResult {
        needs_upgrade: result.needs_upgrade,
        old_version: result.old_version,
        object_stores: result.object_stores,
    })
}

/// Close an IndexedDB database.
#[op2(fast)]
fn op_idb_close(state: &mut OpState, #[string] name: &str) {
    let storage = state.borrow::<IdbStorage>();
    storage.close_database(name);
}

/// Delete an IndexedDB database.
#[op2(async)]
async fn op_idb_delete_database(
    state: Rc<RefCell<OpState>>,
    #[string] name: String,
) -> Result<(), deno_core::error::AnyError> {
    let storage = {
        let state = state.borrow();
        state.borrow::<IdbStorage>().clone()
    };

    storage.delete_database(&name).await.map_err(|e| {
        deno_core::error::generic_error(format!("IDB delete database failed: {}", e))
    })
}

#[derive(Deserialize)]
struct CreateObjectStoreOptions {
    #[serde(rename = "keyPath")]
    key_path: Option<String>,
    #[serde(rename = "autoIncrement")]
    auto_increment: Option<bool>,
}

/// Create an object store.
#[op2]
fn op_idb_create_object_store(
    state: &mut OpState,
    #[string] db_name: &str,
    #[string] store_name: &str,
    #[serde] options: CreateObjectStoreOptions,
) -> Result<(), deno_core::error::AnyError> {
    let storage = state.borrow::<IdbStorage>();

    storage
        .create_object_store(
            db_name,
            store_name,
            options.key_path.as_deref(),
            options.auto_increment.unwrap_or(false),
        )
        .map_err(|e| deno_core::error::generic_error(format!("Create object store failed: {}", e)))
}

/// Delete an object store.
#[op2(fast)]
fn op_idb_delete_object_store(
    state: &mut OpState,
    #[string] db_name: &str,
    #[string] store_name: &str,
) -> Result<(), deno_core::error::AnyError> {
    let storage = state.borrow::<IdbStorage>();

    storage
        .delete_object_store(db_name, store_name)
        .map_err(|e| deno_core::error::generic_error(format!("Delete object store failed: {}", e)))
}

/// Get a value from an object store.
#[op2(async)]
#[serde]
async fn op_idb_get(
    state: Rc<RefCell<OpState>>,
    #[string] db_name: String,
    #[string] store_name: String,
    #[serde] key: serde_json::Value,
) -> Result<Option<serde_json::Value>, deno_core::error::AnyError> {
    let storage = {
        let state = state.borrow();
        state.borrow::<IdbStorage>().clone()
    };

    storage.get(&db_name, &store_name, &key).await.map_err(|e| {
        deno_core::error::generic_error(format!("IDB get failed: {}", e))
    })
}

/// Put a value into an object store.
#[op2(async)]
async fn op_idb_put(
    state: Rc<RefCell<OpState>>,
    #[string] db_name: String,
    #[string] store_name: String,
    #[serde] key: serde_json::Value,
    #[serde] value: serde_json::Value,
) -> Result<(), deno_core::error::AnyError> {
    let storage = {
        let state = state.borrow();
        state.borrow::<IdbStorage>().clone()
    };

    storage.put(&db_name, &store_name, &key, &value).await.map_err(|e| {
        deno_core::error::generic_error(format!("IDB put failed: {}", e))
    })
}

/// Delete a value from an object store.
#[op2(async)]
async fn op_idb_delete(
    state: Rc<RefCell<OpState>>,
    #[string] db_name: String,
    #[string] store_name: String,
    #[serde] key: serde_json::Value,
) -> Result<(), deno_core::error::AnyError> {
    let storage = {
        let state = state.borrow();
        state.borrow::<IdbStorage>().clone()
    };

    storage.delete(&db_name, &store_name, &key).await.map_err(|e| {
        deno_core::error::generic_error(format!("IDB delete failed: {}", e))
    })
}

/// Clear all values from an object store.
#[op2(async)]
async fn op_idb_clear(
    state: Rc<RefCell<OpState>>,
    #[string] db_name: String,
    #[string] store_name: String,
) -> Result<(), deno_core::error::AnyError> {
    let storage = {
        let state = state.borrow();
        state.borrow::<IdbStorage>().clone()
    };

    storage.clear(&db_name, &store_name).await.map_err(|e| {
        deno_core::error::generic_error(format!("IDB clear failed: {}", e))
    })
}

/// Get all values from an object store.
#[op2(async)]
#[serde]
async fn op_idb_get_all(
    state: Rc<RefCell<OpState>>,
    #[string] db_name: String,
    #[string] store_name: String,
    count: Option<u32>,
) -> Result<Vec<serde_json::Value>, deno_core::error::AnyError> {
    let storage = {
        let state = state.borrow();
        state.borrow::<IdbStorage>().clone()
    };

    storage.get_all(&db_name, &store_name, count).await.map_err(|e| {
        deno_core::error::generic_error(format!("IDB get_all failed: {}", e))
    })
}

/// Get all keys from an object store.
#[op2(async)]
#[serde]
async fn op_idb_get_all_keys(
    state: Rc<RefCell<OpState>>,
    #[string] db_name: String,
    #[string] store_name: String,
    count: Option<u32>,
) -> Result<Vec<serde_json::Value>, deno_core::error::AnyError> {
    let storage = {
        let state = state.borrow();
        state.borrow::<IdbStorage>().clone()
    };

    storage.get_all_keys(&db_name, &store_name, count).await.map_err(|e| {
        deno_core::error::generic_error(format!("IDB get_all_keys failed: {}", e))
    })
}

/// Count values in an object store.
#[op2(async)]
async fn op_idb_count(
    state: Rc<RefCell<OpState>>,
    #[string] db_name: String,
    #[string] store_name: String,
) -> Result<u32, deno_core::error::AnyError> {
    let storage = {
        let state = state.borrow();
        state.borrow::<IdbStorage>().clone()
    };

    storage.count(&db_name, &store_name).await.map_err(|e| {
        deno_core::error::generic_error(format!("IDB count failed: {}", e))
    })
}

// ============================================================================
// Skill Bridge Ops (db, store, net)
// ============================================================================

#[op2]
#[bigint]
fn op_db_exec(
    state: &mut OpState,
    #[string] sql: &str,
    #[string] params_json: Option<String>,
) -> Result<i64, deno_core::error::AnyError> {
    let storage = state.borrow::<IdbStorage>();
    let ctx = state.borrow::<SkillContext>();

    let params: Vec<serde_json::Value> = match params_json {
        Some(p) => serde_json::from_str(&p).unwrap_or_default(),
        None => Vec::new(),
    };

    storage
        .skill_db_exec(&ctx.skill_id, sql, &params)
        .map(|n| n as i64)
        .map_err(|e| deno_core::error::generic_error(e))
}

#[op2]
#[string]
fn op_db_get(
    state: &mut OpState,
    #[string] sql: &str,
    #[string] params_json: Option<String>,
) -> Result<String, deno_core::error::AnyError> {
    let storage = state.borrow::<IdbStorage>();
    let ctx = state.borrow::<SkillContext>();

    let params: Vec<serde_json::Value> = match params_json {
        Some(p) => serde_json::from_str(&p).unwrap_or_default(),
        None => Vec::new(),
    };

    storage
        .skill_db_get(&ctx.skill_id, sql, &params)
        .map(|v| v.to_string())
        .map_err(|e| deno_core::error::generic_error(e))
}

#[op2]
#[string]
fn op_db_all(
    state: &mut OpState,
    #[string] sql: &str,
    #[string] params_json: Option<String>,
) -> Result<String, deno_core::error::AnyError> {
    let storage = state.borrow::<IdbStorage>();
    let ctx = state.borrow::<SkillContext>();

    let params: Vec<serde_json::Value> = match params_json {
        Some(p) => serde_json::from_str(&p).unwrap_or_default(),
        None => Vec::new(),
    };

    storage
        .skill_db_all(&ctx.skill_id, sql, &params)
        .map(|v| v.to_string())
        .map_err(|e| deno_core::error::generic_error(e))
}

#[op2]
#[string]
fn op_db_kv_get(state: &mut OpState, #[string] key: &str) -> Result<String, deno_core::error::AnyError> {
    let storage = state.borrow::<IdbStorage>();
    let ctx = state.borrow::<SkillContext>();

    storage
        .skill_kv_get(&ctx.skill_id, key)
        .map(|v| v.to_string())
        .map_err(|e| deno_core::error::generic_error(e))
}

#[op2(fast)]
fn op_db_kv_set(
    state: &mut OpState,
    #[string] key: &str,
    #[string] value_json: &str,
) -> Result<(), deno_core::error::AnyError> {
    let storage = state.borrow::<IdbStorage>();
    let ctx = state.borrow::<SkillContext>();

    let value: serde_json::Value =
        serde_json::from_str(value_json).unwrap_or(serde_json::Value::Null);

    storage
        .skill_kv_set(&ctx.skill_id, key, &value)
        .map_err(|e| deno_core::error::generic_error(e))
}

#[op2]
#[string]
fn op_store_get(state: &mut OpState, #[string] key: &str) -> Result<String, deno_core::error::AnyError> {
    let storage = state.borrow::<IdbStorage>();
    let ctx = state.borrow::<SkillContext>();

    storage
        .skill_store_get(&ctx.skill_id, key)
        .map(|v| v.to_string())
        .map_err(|e| deno_core::error::generic_error(e))
}

#[op2(fast)]
fn op_store_set(
    state: &mut OpState,
    #[string] key: &str,
    #[string] value_json: &str,
) -> Result<(), deno_core::error::AnyError> {
    let storage = state.borrow::<IdbStorage>();
    let ctx = state.borrow::<SkillContext>();

    let value: serde_json::Value =
        serde_json::from_str(value_json).unwrap_or(serde_json::Value::Null);

    storage
        .skill_store_set(&ctx.skill_id, key, &value)
        .map_err(|e| deno_core::error::generic_error(e))
}

#[op2(fast)]
fn op_store_delete(state: &mut OpState, #[string] key: &str) -> Result<(), deno_core::error::AnyError> {
    let storage = state.borrow::<IdbStorage>();
    let ctx = state.borrow::<SkillContext>();

    storage
        .skill_store_delete(&ctx.skill_id, key)
        .map_err(|e| deno_core::error::generic_error(e))
}

#[op2]
#[string]
fn op_store_keys(state: &mut OpState) -> Result<String, deno_core::error::AnyError> {
    let storage = state.borrow::<IdbStorage>();
    let ctx = state.borrow::<SkillContext>();

    storage
        .skill_store_keys(&ctx.skill_id)
        .map(|keys| serde_json::to_string(&keys).unwrap_or_else(|_| "[]".to_string()))
        .map_err(|e| deno_core::error::generic_error(e))
}

#[op2]
#[string]
fn op_net_fetch(
    #[string] url: &str,
    #[string] options_json: &str,
) -> Result<String, deno_core::error::AnyError> {
    crate::runtime::bridge::net::http_fetch(url, options_json)
        .map_err(|e| deno_core::error::generic_error(e))
}

// ============================================================================
// State Bridge Ops
// ============================================================================

/// Get a value from the skill's published state.
#[op2]
#[string]
fn op_state_get(state: &mut OpState, #[string] key: &str) -> Result<String, deno_core::error::AnyError> {
    let ctx = state.borrow::<SkillContext>();

    let skill_state = ctx.skill_state.as_ref().ok_or_else(|| {
        deno_core::error::generic_error("Skill state not initialized")
    })?;

    let published_state = &skill_state.read().published_state;
    let value = published_state.get(key).cloned().unwrap_or(serde_json::Value::Null);
    Ok(value.to_string())
}

/// Set a value in the skill's published state.
#[op2(fast)]
fn op_state_set(
    state: &mut OpState,
    #[string] key: &str,
    #[string] value_json: &str,
) -> Result<(), deno_core::error::AnyError> {
    let ctx = state.borrow::<SkillContext>();

    let skill_state = ctx.skill_state.as_ref().ok_or_else(|| {
        deno_core::error::generic_error("Skill state not initialized")
    })?;

    let value: serde_json::Value =
        serde_json::from_str(value_json).unwrap_or(serde_json::Value::Null);
    skill_state.write().published_state.insert(key.to_string(), value);
    Ok(())
}

/// Merge a partial object into the skill's published state.
#[op2(fast)]
fn op_state_set_partial(
    state: &mut OpState,
    #[string] partial_json: &str,
) -> Result<(), deno_core::error::AnyError> {
    let ctx = state.borrow::<SkillContext>();

    let skill_state = ctx.skill_state.as_ref().ok_or_else(|| {
        deno_core::error::generic_error("Skill state not initialized")
    })?;

    let partial: serde_json::Value =
        serde_json::from_str(partial_json).unwrap_or(serde_json::Value::Object(Default::default()));

    if let serde_json::Value::Object(map) = partial {
        let mut state_guard = skill_state.write();
        for (k, v) in map {
            state_guard.published_state.insert(k, v);
        }
    }
    Ok(())
}

// ============================================================================
// Data Bridge Ops
// ============================================================================

/// Read a file from the skill's data directory.
#[op2]
#[string]
fn op_data_read(
    state: &mut OpState,
    #[string] filename: &str,
) -> Result<String, deno_core::error::AnyError> {
    let ctx = state.borrow::<SkillContext>();

    let data_dir = ctx.data_dir.as_ref().ok_or_else(|| {
        deno_core::error::generic_error("Data directory not configured")
    })?;

    let path = data_dir.join(filename);

    // Prevent path traversal
    if !path.starts_with(data_dir) {
        return Err(deno_core::error::generic_error("Invalid filename: path traversal"));
    }

    std::fs::read_to_string(&path).map_err(|e| {
        deno_core::error::generic_error(format!("Failed to read file '{}': {}", filename, e))
    })
}

/// Write a file to the skill's data directory.
#[op2(fast)]
fn op_data_write(
    state: &mut OpState,
    #[string] filename: &str,
    #[string] content: &str,
) -> Result<(), deno_core::error::AnyError> {
    let ctx = state.borrow::<SkillContext>();

    let data_dir = ctx.data_dir.as_ref().ok_or_else(|| {
        deno_core::error::generic_error("Data directory not configured")
    })?;

    let path = data_dir.join(filename);

    // Prevent path traversal
    if !path.starts_with(data_dir) {
        return Err(deno_core::error::generic_error("Invalid filename: path traversal"));
    }

    // Ensure parent directories exist
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            deno_core::error::generic_error(format!("Failed to create directory: {}", e))
        })?;
    }

    std::fs::write(&path, content).map_err(|e| {
        deno_core::error::generic_error(format!("Failed to write file '{}': {}", filename, e))
    })
}

// ============================================================================
// TDLib Ops (telegram skill only)
// ============================================================================

/// Check if the current skill is the telegram skill.
fn check_telegram_skill(state: &OpState) -> Result<(), deno_core::error::AnyError> {
    let ctx = state.borrow::<SkillContext>();
    if ctx.skill_id != "telegram" {
        return Err(deno_core::error::generic_error(
            "TDLib is only available to the telegram skill",
        ));
    }
    Ok(())
}

/// Check if TDLib is available (always true on desktop).
#[op2(fast)]
fn op_tdlib_is_available() -> bool {
    true
}

/// Create a TDLib client with the given data directory.
/// This is synchronous since create_client just spawns a worker thread and returns.
#[op2(fast)]
fn op_tdlib_create_client(
    state: &mut OpState,
    #[string] data_dir: String,
) -> Result<i32, deno_core::error::AnyError> {
    // Check skill permission
    check_telegram_skill(state)?;

    let path = std::path::PathBuf::from(data_dir);

    crate::services::tdlib::TDLIB_MANAGER
        .create_client(path)
        .map_err(|e| deno_core::error::generic_error(e))
}

/// Send a request to TDLib and wait for the response.
#[op2(async)]
#[serde]
async fn op_tdlib_send(
    state: Rc<RefCell<OpState>>,
    #[serde] request: serde_json::Value,
) -> Result<serde_json::Value, deno_core::error::AnyError> {
    // Check skill permission
    {
        let state = state.borrow();
        check_telegram_skill(&state)?;
    }

    crate::services::tdlib::TDLIB_MANAGER
        .send(request)
        .await
        .map_err(|e| deno_core::error::generic_error(e))
}

/// Receive the next update from TDLib (with timeout in ms).
#[op2(async)]
#[serde]
async fn op_tdlib_receive(
    state: Rc<RefCell<OpState>>,
    timeout_ms: u32,
) -> Result<Option<serde_json::Value>, deno_core::error::AnyError> {
    // Check skill permission
    {
        let state = state.borrow();
        check_telegram_skill(&state)?;
    }

    Ok(crate::services::tdlib::TDLIB_MANAGER.receive(timeout_ms).await)
}

/// Destroy the TDLib client and clean up resources.
#[op2(async)]
async fn op_tdlib_destroy(
    state: Rc<RefCell<OpState>>,
) -> Result<(), deno_core::error::AnyError> {
    // Check skill permission
    {
        let state = state.borrow();
        check_telegram_skill(&state)?;
    }

    crate::services::tdlib::TDLIB_MANAGER
        .destroy()
        .await
        .map_err(|e| deno_core::error::generic_error(e))
}

// ============================================================================
// Model Ops (local LLM inference)
// ============================================================================

/// Check if local model API is available (desktop only).
#[op2(fast)]
fn op_model_is_available() -> bool {
    true
}

/// Get model status (loading, ready, error).
#[op2]
#[serde]
fn op_model_get_status() -> serde_json::Value {
    let status = crate::services::llama::LLAMA_MANAGER.get_status();
    serde_json::to_value(status).unwrap_or_default()
}

/// Generate text from prompt (async, blocking inference on thread pool).
#[op2(async)]
#[string]
async fn op_model_generate(
    #[string] prompt: String,
    #[serde] config: serde_json::Value,
) -> Result<String, deno_core::error::AnyError> {
    let cfg: crate::services::llama::GenerateConfig =
        serde_json::from_value(config).unwrap_or_default();

    crate::services::llama::LLAMA_MANAGER
        .generate(&prompt, cfg)
        .await
        .map_err(|e| deno_core::error::generic_error(e))
}

/// Summarize text (async).
#[op2(async)]
#[string]
async fn op_model_summarize(
    #[string] text: String,
    max_tokens: u32,
) -> Result<String, deno_core::error::AnyError> {
    crate::services::llama::LLAMA_MANAGER
        .summarize(&text, max_tokens)
        .await
        .map_err(|e| deno_core::error::generic_error(e))
}
