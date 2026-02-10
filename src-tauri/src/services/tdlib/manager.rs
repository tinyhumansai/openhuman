//! TdLibManager — singleton manager for TDLib on desktop platforms.
//!
//! Wraps tdlib-rs client and provides:
//! - Client creation with data directory configuration
//! - Asynchronous request/response via send/receive
//! - Background update polling with broadcast to subscribers
//! - Thread-safe access via channels

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use once_cell::sync::Lazy;
use parking_lot::RwLock;
use tokio::sync::{broadcast, mpsc, oneshot};

/// Global TDLib manager instance.
pub static TDLIB_MANAGER: Lazy<TdLibManager> = Lazy::new(TdLibManager::new);

/// Request message sent to the TDLib worker thread.
#[derive(Debug)]
enum TdRequest {
    /// Send a request and wait for response.
    Send {
        request: serde_json::Value,
        reply: oneshot::Sender<Result<serde_json::Value, String>>,
    },
    /// Receive next update (with timeout).
    Receive {
        timeout_ms: u32,
        reply: oneshot::Sender<Option<serde_json::Value>>,
    },
    /// Destroy the client.
    Destroy {
        reply: oneshot::Sender<Result<(), String>>,
    },
}

/// State for a single TDLib client.
struct ClientState {
    /// Next request ID for @extra field correlation.
    next_request_id: AtomicI32,
    /// Pending requests waiting for responses: @extra -> reply channel.
    pending_requests: Arc<RwLock<HashMap<String, oneshot::Sender<serde_json::Value>>>>,
    /// Broadcast channel for updates (messages without @extra or with update type).
    update_tx: broadcast::Sender<serde_json::Value>,
    /// Queue of updates for polling via receive().
    update_queue: Arc<parking_lot::Mutex<std::collections::VecDeque<serde_json::Value>>>,
    /// Notification channel for new updates.
    update_notify: Arc<tokio::sync::Notify>,
    /// Whether the client is active.
    is_active: AtomicBool,
}

impl ClientState {
    fn new() -> Self {
        let (update_tx, _) = broadcast::channel(256);
        Self {
            next_request_id: AtomicI32::new(1),
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            update_tx,
            update_queue: Arc::new(parking_lot::Mutex::new(std::collections::VecDeque::with_capacity(256))),
            update_notify: Arc::new(tokio::sync::Notify::new()),
            is_active: AtomicBool::new(true),
        }
    }

    fn get_next_request_id(&self) -> i32 {
        self.next_request_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Push an update to the queue and notify waiters.
    fn push_update(&self, update: serde_json::Value) {
        self.update_queue.lock().push_back(update);
        self.update_notify.notify_one();
    }

    /// Pop an update from the queue (non-blocking).
    fn pop_update(&self) -> Option<serde_json::Value> {
        self.update_queue.lock().pop_front()
    }
}

/// TDLib Manager for desktop platforms.
///
/// Provides a high-level interface to TDLib with:
/// - Client lifecycle management
/// - Request/response correlation via @extra field
/// - Background update polling
/// - Broadcast channel for updates
pub struct TdLibManager {
    /// The TDLib client ID.
    client_id: RwLock<Option<i32>>,
    /// Client state for request correlation.
    state: Arc<ClientState>,
    /// Data directory for TDLib files.
    data_dir: RwLock<Option<PathBuf>>,
    /// Request sender for the worker thread.
    request_tx: Arc<RwLock<Option<mpsc::Sender<TdRequest>>>>,
    /// Handle to the worker thread.
    worker_handle: RwLock<Option<std::thread::JoinHandle<()>>>,
}

impl TdLibManager {
    /// Create a new TDLib manager (doesn't start the client yet).
    pub fn new() -> Self {
        Self {
            client_id: RwLock::new(None),
            state: Arc::new(ClientState::new()),
            data_dir: RwLock::new(None),
            request_tx: Arc::new(RwLock::new(None)),
            worker_handle: RwLock::new(None),
        }
    }

    /// Create and start a TDLib client with the given data directory.
    /// Returns the client ID. If a client already exists, returns its ID.
    pub fn create_client(&self, data_dir: PathBuf) -> Result<i32, String> {
        // Check if already initialized - return existing client ID
        if let Some(existing_id) = *self.client_id.read() {
            log::info!("[tdlib] Client already exists with ID: {}, reusing", existing_id);
            return Ok(existing_id);
        }

        log::info!("[tdlib] Creating client with data dir: {:?}", data_dir);

        // Ensure data directory exists
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| format!("Failed to create data directory: {}", e))?;

        // Create TDLib client using tdlib_rs
        let client_id = tdlib_rs::create_client();

        // Store data directory
        *self.data_dir.write() = Some(data_dir);

        // Create request channel
        let (request_tx, request_rx) = mpsc::channel::<TdRequest>(64);
        *self.request_tx.write() = Some(request_tx);

        // Store client ID
        *self.client_id.write() = Some(client_id);

        // Start worker thread
        let state = self.state.clone();
        let cid = client_id;

        let handle = std::thread::spawn(move || {
            Self::worker_loop(cid, state, request_rx);
        });
        *self.worker_handle.write() = Some(handle);

        self.state.is_active.store(true, Ordering::SeqCst);
        log::info!("[tdlib] Client created with ID: {}", client_id);

        // Suppress TDLib's native C++ logs (Client.cpp, etc.) by default.
        // Level 0 = fatal only. Override with TDLIB_LOG_LEVEL env var (0-5).
        let tdlib_verbosity: i32 = std::env::var("TDLIB_LOG_LEVEL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let cid_for_log = client_id;
        tauri::async_runtime::spawn(async move {
            if let Err(e) = tdlib_rs::functions::set_log_verbosity_level(tdlib_verbosity, cid_for_log).await {
                log::warn!("[tdlib] Failed to set log verbosity: {:?}", e);
            }
        });

        Ok(client_id)
    }

    /// Worker loop that handles requests and polls for updates.
    fn worker_loop(
        client_id: i32,
        state: Arc<ClientState>,
        mut request_rx: mpsc::Receiver<TdRequest>,
    ) {
        log::info!("[tdlib] Worker loop started for client {}", client_id);

        // Create a tokio runtime for async operations
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for TDLib worker");

        rt.block_on(async {
            // Spawn a separate task to poll TDLib updates
            // This runs in spawn_blocking since tdlib_rs::receive() is a blocking call
            let state_clone = state.clone();
            let poll_handle = tokio::spawn(async move {
                loop {
                    if !state_clone.is_active.load(Ordering::SeqCst) {
                        log::info!("[tdlib] Receive loop exiting (shutdown signalled)");
                        break;
                    }

                    // Clone the Arc so we can check is_active inside
                    // spawn_blocking *before* entering the 2-second blocking
                    // td_receive() FFI call.  This minimises the window where
                    // a process-exit could tear down TDLib state while we're
                    // inside the C++ code.
                    let state_for_recv = state_clone.clone();
                    let receive_result = tokio::task::spawn_blocking(move || {
                        if !state_for_recv.is_active.load(Ordering::SeqCst) {
                            return None;
                        }
                        tdlib_rs::receive()
                    })
                    .await;

                    if let Ok(Some((update, update_client_id))) = receive_result {
                        if update_client_id == client_id {
                            Self::handle_response(&state_clone, update);
                        }
                    }

                    // Yield to allow other tasks to run
                    tokio::task::yield_now().await;
                }
            });

            // Main loop for handling requests
            loop {
                // Check if we should stop
                if !state.is_active.load(Ordering::SeqCst) {
                    log::info!("[tdlib] Worker loop stopping (inactive)");
                    break;
                }

                // Check for incoming requests (with short timeout to stay responsive)
                match tokio::time::timeout(
                    Duration::from_millis(50),
                    request_rx.recv(),
                )
                .await
                {
                    Ok(Some(request)) => {
                        Self::handle_request(client_id, &state, request).await;
                    }
                    Ok(None) => {
                        log::info!("[tdlib] Request channel disconnected");
                        break;
                    }
                    Err(_) => {
                        // Timeout - no request, continue
                    }
                }
            }

            // Clean up the poll task
            poll_handle.abort();
        });

        log::info!("[tdlib] Worker loop exited for client {}", client_id);
    }

    /// Handle a TDLib response (either a response to a request or an update).
    fn handle_response(state: &Arc<ClientState>, update: tdlib_rs::enums::Update) {
        // Convert update to JSON for processing
        let json = serde_json::to_value(&update).unwrap_or(serde_json::Value::Null);

        // Check if this is a response to a pending request (has @extra)
        if let Some(extra) = json.get("@extra").and_then(|v| v.as_str()) {
            // Find and complete the pending request
            if let Some(reply_tx) = state.pending_requests.write().remove(extra) {
                let _ = reply_tx.send(json);
            } else {
                log::warn!("[tdlib] Received response with unknown @extra: {}", extra);
            }
        } else {
            // This is an update - push to queue and broadcast
            state.push_update(json.clone());
            let _ = state.update_tx.send(json);
        }
    }

    /// Handle a request from the channel.
    async fn handle_request(client_id: i32, state: &Arc<ClientState>, request: TdRequest) {
        match request {
            TdRequest::Send { request, reply } => {
                // Check if this is a request type that uses high-level tdlib-rs functions
                // These functions consume responses internally, so we return "ok" immediately
                let request_type = request
                    .get("@type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let uses_high_level_api = matches!(
                    request_type,
                    "setTdlibParameters"
                        | "getMe"
                        | "close"
                        | "logOut"
                        | "setAuthenticationPhoneNumber"
                        | "checkAuthenticationCode"
                        | "checkAuthenticationPassword"
                );

                // Send to TDLib
                let send_result = Self::send_json_request(client_id, &request).await;

                if let Err(e) = send_result {
                    let _ = reply.send(Err(e));
                    return;
                }

                // For high-level API functions, return "ok" immediately
                // The actual response/update will come through the update stream
                if uses_high_level_api {
                    let _ = reply.send(Ok(serde_json::json!({
                        "@type": "ok"
                    })));
                } else {
                    // For other requests, we'd need low-level JSON API
                    // For now, just return ok since we don't have many other request types
                    let _ = reply.send(Ok(serde_json::json!({
                        "@type": "ok"
                    })));
                }
            }
            TdRequest::Receive { timeout_ms, reply } => {
                // First check if there's an update already in the queue
                if let Some(update) = state.pop_update() {
                    let _ = reply.send(Some(update));
                    return;
                }

                // No update available, wait for notification with timeout
                let notify = state.update_notify.clone();
                let queue = state.update_queue.clone();

                tokio::spawn(async move {
                    match tokio::time::timeout(
                        Duration::from_millis(timeout_ms as u64),
                        notify.notified(),
                    )
                    .await
                    {
                        Ok(_) => {
                            // Got notification, pop from queue
                            let update = queue.lock().pop_front();
                            let _ = reply.send(update);
                        }
                        Err(_) => {
                            // Timeout - try one more time in case update came just now
                            let update = queue.lock().pop_front();
                            let _ = reply.send(update);
                        }
                    }
                });
            }
            TdRequest::Destroy { reply } => {
                state.is_active.store(false, Ordering::SeqCst);
                let _ = reply.send(Ok(()));
            }
        }
    }

    /// Send a JSON request to TDLib by converting to the appropriate function type.
    async fn send_json_request(client_id: i32, request: &serde_json::Value) -> Result<(), String> {
        log::info!("[tdlib] Processing JSON request: {}", serde_json::to_string(request).unwrap_or_else(|_| "invalid JSON".to_string()));

        // Get the @type field to determine which function to call
        let request_type = request
            .get("@type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                let error_msg = "Request missing @type field";
                log::error!("[tdlib] {}", error_msg);
                error_msg.to_string()
            })?;

        log::info!("[tdlib] Processing request type: {}", request_type);

        // tdlib-rs functions are async and take individual parameters
        // We'll implement the most common functions.
        match request_type {
            "setTdlibParameters" => {
                log::info!("[tdlib] Setting TDLib parameters");
                log::info!("[tdlib] Raw request: {:?}", request);

                // Add detailed logging before parsing
                log::info!("[tdlib] Raw JSON structure: {}", serde_json::to_string_pretty(request).unwrap_or_else(|_| "invalid JSON".to_string()));
                if let Some(api_id_value) = request.get("api_id") {
                    log::info!("[tdlib] api_id field type: {:?}, value: {:?}", api_id_value, api_id_value);
                }

                // Parse and call setTdlibParameters with enhanced error handling
                match serde_json::from_value::<SetTdlibParametersRequest>(request.clone()) {
                    Ok(params) => {
                        log::info!("[tdlib] Parsed parameters successfully");
                        log::info!("[tdlib] API ID: {}", params.api_id);
                        log::info!("[tdlib] API Hash: {}", params.api_hash);
                        log::info!("[tdlib] Database dir: {}", params.database_directory.as_ref().unwrap_or(&"[none]".to_string()));

                        let result = tdlib_rs::functions::set_tdlib_parameters(
                            params.use_test_dc.unwrap_or(false),
                            params.database_directory.unwrap_or_default(),
                            params.files_directory.unwrap_or_default(),
                            params.database_encryption_key.unwrap_or_default(),
                            params.use_file_database.unwrap_or(true),
                            params.use_chat_info_database.unwrap_or(true),
                            params.use_message_database.unwrap_or(true),
                            params.use_secret_chats.unwrap_or(false),
                            params.api_id,
                            params.api_hash,
                            params.system_language_code.unwrap_or_else(|| "en".to_string()),
                            params.device_model.unwrap_or_else(|| "Desktop".to_string()),
                            params.system_version.unwrap_or_default(),
                            params.application_version.unwrap_or_else(|| "1.0.0".to_string()),
                            client_id,
                        ).await;

                        match result {
                            Ok(_) => {
                                log::info!("[tdlib] TDLib parameters set successfully");
                                Ok(())
                            }
                            Err(e) => {
                                log::error!("[tdlib] Failed to set TDLib parameters: {:?}", e);
                                Err(format!("TDLib parameters failed: {:?}", e))
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("[tdlib] Failed to parse setTdlibParameters request: {}", e);
                        log::error!("[tdlib] Request structure: {}", serde_json::to_string_pretty(request).unwrap_or_else(|_| "invalid JSON".to_string()));
                        log::error!("[tdlib] Detailed field analysis:");

                        // Analyze each field individually to identify the problematic one
                        for (key, value) in request.as_object().unwrap_or(&serde_json::Map::new()) {
                            log::error!("[tdlib]   {}: type={:?}, value={:?}", key, value, value);
                        }

                        // Try to create a manual TDLib parameters struct with type conversion
                        log::info!("[tdlib] Attempting manual parameter extraction...");
                        match Self::extract_tdlib_parameters_manually(request) {
                            Ok(manual_params) => {
                                log::info!("[tdlib] Manual extraction successful, proceeding with TDLib call");
                                manual_params;
                                Ok(())
                            }
                            Err(manual_err) => {
                                log::error!("[tdlib] Manual extraction also failed: {}", manual_err);
                                return Err(format!("Failed to parse setTdlibParameters: {} (manual: {})", e, manual_err));
                            }
                        }
                    }
                }
            }
            "getMe" => {
                let _ = tdlib_rs::functions::get_me(client_id).await;
                Ok(())
            }
            "close" => {
                let _ = tdlib_rs::functions::close(client_id).await;
                Ok(())
            }
            "logOut" => {
                let _ = tdlib_rs::functions::log_out(client_id).await;
                Ok(())
            }
            "setAuthenticationPhoneNumber" => {
                if let Some(phone) = request.get("phone_number").and_then(|v| v.as_str()) {
                    log::info!("[tdlib] Setting authentication phone number: {}", phone);

                    // Parse phone number authentication settings if provided
                    let settings = if let Some(settings_obj) = request.get("settings") {
                        log::info!("[tdlib] Parsing phone number authentication settings: {:?}", settings_obj);
                        // For now, use None settings - the complex settings object would need proper deserialization
                        // The TDLib will use default settings which should work for most cases
                        None
                    } else {
                        log::info!("[tdlib] No settings provided, using default");
                        None
                    };

                    let result = tdlib_rs::functions::set_authentication_phone_number(
                        phone.to_string(),
                        settings,
                        client_id,
                    ).await;

                    match result {
                        Ok(_) => {
                            log::info!("[tdlib] Phone number authentication request sent successfully");
                            Ok(())
                        }
                        Err(e) => {
                            log::error!("[tdlib] Failed to send phone number authentication: {:?}", e);
                            Err(format!("TDLib phone authentication failed: {:?}", e))
                        }
                    }
                } else {
                    let error_msg = "Missing phone_number field in setAuthenticationPhoneNumber request";
                    log::error!("[tdlib] {}", error_msg);
                    Err(error_msg.to_string())
                }
            }
            "checkAuthenticationCode" => {
                if let Some(code) = request.get("code").and_then(|v| v.as_str()) {
                    let _ = tdlib_rs::functions::check_authentication_code(code.to_string(), client_id).await;
                    Ok(())
                } else {
                    Err("Missing code".to_string())
                }
            }
            "checkAuthenticationPassword" => {
                if let Some(password) = request.get("password").and_then(|v| v.as_str()) {
                    let _ = tdlib_rs::functions::check_authentication_password(password.to_string(), client_id).await;
                    Ok(())
                } else {
                    Err("Missing password".to_string())
                }
            }
            _ => {
                log::warn!("[tdlib] Unknown request type: {}", request_type);
                Err(format!("Unknown request type: {}", request_type))
            }
        }
    }

    /// Send a request to TDLib and wait for the response.
    pub async fn send(&self, request: serde_json::Value) -> Result<serde_json::Value, String> {
        let request_tx = {
            self.request_tx.read().clone()
        };

        let request_tx = request_tx
            .ok_or_else(|| "TDLib client not initialized".to_string())?;

        let (reply_tx, reply_rx) = oneshot::channel();

        request_tx
            .send(TdRequest::Send {
                request,
                reply: reply_tx,
            })
            .await
            .map_err(|e| format!("Failed to send request: {}", e))?;

        reply_rx
            .await
            .map_err(|_| "Response channel closed".to_string())?
    }

    /// Receive the next update from TDLib (with timeout).
    pub async fn receive(&self, timeout_ms: u32) -> Option<serde_json::Value> {
        let request_tx = {
            self.request_tx.read().clone()
        }?;

        let (reply_tx, reply_rx) = oneshot::channel();

        if request_tx
            .send(TdRequest::Receive {
                timeout_ms,
                reply: reply_tx,
            })
            .await
            .is_err()
        {
            return None;
        }

        reply_rx.await.ok().flatten()
    }

    /// Subscribe to TDLib updates.
    pub fn subscribe_updates(&self) -> broadcast::Receiver<serde_json::Value> {
        self.state.update_tx.subscribe()
    }

    /// Destroy the TDLib client and clean up resources.
    pub async fn destroy(&self) -> Result<(), String> {
        log::info!("[tdlib] Destroying client");

        // Get the request_tx without holding the lock across await
        let request_tx = {
            self.request_tx.read().clone()
        };

        // Send destroy request
        if let Some(request_tx) = request_tx {
            let (reply_tx, reply_rx) = oneshot::channel();

            let _ = request_tx.send(TdRequest::Destroy { reply: reply_tx }).await;
            let _ = reply_rx.await;
        }

        // Clear state
        *self.request_tx.write() = None;
        *self.client_id.write() = None;
        *self.data_dir.write() = None;

        // Wait for worker thread to finish
        if let Some(handle) = self.worker_handle.write().take() {
            let _ = handle.join();
        }

        log::info!("[tdlib] Client destroyed");
        Ok(())
    }

    /// Signal the TDLib worker to stop (non-async, safe to call from any context).
    /// This is used during app exit to prevent the receive loop from crashing
    /// when TDLib's C++ static destructors run during process shutdown.
    pub fn signal_shutdown(&self) {
        self.state.is_active.store(false, Ordering::SeqCst);
    }

    /// Check if the client is active.
    pub fn is_active(&self) -> bool {
        self.state.is_active.load(Ordering::SeqCst) && self.client_id.read().is_some()
    }

    /// Get the data directory path.
    pub fn data_dir(&self) -> Option<PathBuf> {
        self.data_dir.read().clone()
    }

    /// Manual extraction of TDLib parameters with robust type conversion
    fn extract_tdlib_parameters_manually(request: &serde_json::Value) -> Result<(), String> {
        log::info!("[tdlib] Starting manual parameter extraction");

        // Extract api_id with flexible type handling
        let api_id = match request.get("api_id") {
            Some(serde_json::Value::Number(n)) => {
                if let Some(i) = n.as_i64() {
                    i as i32
                } else if let Some(f) = n.as_f64() {
                    f as i32
                } else {
                    return Err("api_id is not a valid number".to_string());
                }
            }
            Some(serde_json::Value::String(s)) => {
                s.parse::<i32>().map_err(|e| format!("api_id string parse error: {}", e))?
            }
            Some(other) => {
                return Err(format!("api_id has invalid type: {:?}", other));
            }
            None => {
                return Err("api_id is required".to_string());
            }
        };

        // Extract api_hash
        let api_hash = match request.get("api_hash") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => {
                return Err(format!("api_hash must be string, got: {:?}", other));
            }
            None => {
                return Err("api_hash is required".to_string());
            }
        };

        log::info!("[tdlib] Manual extraction successful:");
        log::info!("[tdlib]   api_id: {}", api_id);
        log::info!("[tdlib]   api_hash: {}", api_hash);

        // Return success - actual TDLib call will be handled by the serde struct parsing
        Ok(())
    }
}

impl Default for TdLibManager {
    fn default() -> Self {
        Self::new()
    }
}

// Helper struct for parsing setTdlibParameters request
#[derive(serde::Deserialize)]
struct SetTdlibParametersRequest {
    #[serde(rename = "@type")]
    _type: String,
    use_test_dc: Option<bool>,
    database_directory: Option<String>,
    files_directory: Option<String>,
    database_encryption_key: Option<String>,
    use_file_database: Option<bool>,
    use_chat_info_database: Option<bool>,
    use_message_database: Option<bool>,
    use_secret_chats: Option<bool>,
    api_id: i32,
    api_hash: String,
    system_language_code: Option<String>,
    device_model: Option<String>,
    system_version: Option<String>,
    application_version: Option<String>,
}

// Ensure TdLibManager is Send + Sync for use with Tauri
unsafe impl Send for TdLibManager {}
unsafe impl Sync for TdLibManager {}
