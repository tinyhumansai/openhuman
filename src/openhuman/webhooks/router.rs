//! Webhook router — maps tunnel UUIDs to owning skills with isolation enforcement.

use super::types::{
    TunnelRegistration, WebhookDebugEvent, WebhookDebugLogEntry, WebhookRequest,
    WebhookResponseData,
};
use crate::core::event_bus::{publish_global, DomainEvent};
use log::{debug, error, warn};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

const MAX_DEBUG_LOG_ENTRIES: usize = 250;

static WEBHOOK_DEBUG_EVENTS: Lazy<broadcast::Sender<WebhookDebugEvent>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(512);
    tx
});

/// Persistent state serialized to disk.
#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedRoutes {
    registrations: Vec<TunnelRegistration>,
}

/// Routes incoming webhook requests to the skill that owns the tunnel.
///
/// All mutation methods enforce ownership — a skill can only modify its own
/// tunnel registrations and never see or touch another skill's tunnels.
pub struct WebhookRouter {
    /// Keyed by `tunnel_uuid`.
    routes: RwLock<HashMap<String, TunnelRegistration>>,
    /// Recent webhook request/response activity for developer tooling.
    debug_logs: RwLock<VecDeque<WebhookDebugLogEntry>>,
    /// Path to the persistence file (e.g. `~/.openhuman/webhook_routes.json`).
    persist_path: Option<PathBuf>,
}

impl WebhookRouter {
    /// Create a new router, optionally loading persisted routes from disk.
    pub fn new(persist_path: Option<PathBuf>) -> Self {
        let routes = if let Some(ref path) = persist_path {
            match std::fs::read_to_string(path) {
                Ok(data) => match serde_json::from_str::<PersistedRoutes>(&data) {
                    Ok(persisted) => {
                        let map: HashMap<String, TunnelRegistration> = persisted
                            .registrations
                            .into_iter()
                            .map(|r| (r.tunnel_uuid.clone(), r))
                            .collect();
                        debug!(
                            "[webhooks] Loaded {} persisted route(s) from {:?}",
                            map.len(),
                            path
                        );
                        map
                    }
                    Err(e) => {
                        warn!("[webhooks] Failed to parse persisted routes: {}", e);
                        HashMap::new()
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    debug!("[webhooks] No persisted routes file at {:?}", path);
                    HashMap::new()
                }
                Err(e) => {
                    error!(
                        "[webhooks] Failed to read persisted routes at {:?}: {}",
                        path, e
                    );
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        Self {
            routes: RwLock::new(routes),
            debug_logs: RwLock::new(VecDeque::new()),
            persist_path,
        }
    }

    /// Register a tunnel for a skill.
    ///
    /// Rejects the operation if the tunnel UUID is already owned by a
    /// *different* skill. Re-registering from the same skill is a no-op update.
    pub fn register(
        &self,
        tunnel_uuid: &str,
        skill_id: &str,
        tunnel_name: Option<String>,
        backend_tunnel_id: Option<String>,
    ) -> Result<(), String> {
        self.register_target(
            tunnel_uuid,
            "skill",
            skill_id,
            tunnel_name,
            backend_tunnel_id,
        )
    }

    /// Register a built-in echo webhook target for ad-hoc testing.
    pub fn register_echo(
        &self,
        tunnel_uuid: &str,
        tunnel_name: Option<String>,
        backend_tunnel_id: Option<String>,
    ) -> Result<(), String> {
        self.register_target(tunnel_uuid, "echo", "echo", tunnel_name, backend_tunnel_id)
    }

    fn register_target(
        &self,
        tunnel_uuid: &str,
        target_kind: &str,
        skill_id: &str,
        tunnel_name: Option<String>,
        backend_tunnel_id: Option<String>,
    ) -> Result<(), String> {
        let mut routes = self.routes.write().map_err(|e| e.to_string())?;

        if let Some(existing) = routes.get(tunnel_uuid) {
            if existing.skill_id != skill_id || existing.target_kind != target_kind {
                return Err(format!(
                    "Tunnel {} is already owned by {} '{}'; {} '{}' cannot register it",
                    tunnel_uuid, existing.target_kind, existing.skill_id, target_kind, skill_id
                ));
            }
        }

        debug!(
            "[webhooks] Registering tunnel {} → {} '{}'",
            tunnel_uuid, target_kind, skill_id
        );

        let tunnel_name_clone = tunnel_name.clone();
        routes.insert(
            tunnel_uuid.to_string(),
            TunnelRegistration {
                tunnel_uuid: tunnel_uuid.to_string(),
                target_kind: target_kind.to_string(),
                skill_id: skill_id.to_string(),
                tunnel_name,
                backend_tunnel_id,
            },
        );

        drop(routes);
        self.publish_event("registration_changed", None, Some(tunnel_uuid.to_string()));
        self.persist();

        publish_global(DomainEvent::WebhookRegistered {
            tunnel_id: tunnel_uuid.to_string(),
            skill_id: skill_id.to_string(),
            tunnel_name: tunnel_name_clone,
        });

        Ok(())
    }

    /// Unregister a tunnel. Only the owning skill can unregister it.
    pub fn unregister(&self, tunnel_uuid: &str, skill_id: &str) -> Result<(), String> {
        let mut routes = self.routes.write().map_err(|e| e.to_string())?;

        if let Some(existing) = routes.get(tunnel_uuid) {
            if existing.skill_id != skill_id {
                return Err(format!(
                    "Tunnel {} is owned by skill '{}'; skill '{}' cannot unregister it",
                    tunnel_uuid, existing.skill_id, skill_id
                ));
            }
            debug!(
                "[webhooks] Unregistering tunnel {} (skill '{}')",
                tunnel_uuid, skill_id
            );
            routes.remove(tunnel_uuid);
        } else {
            debug!(
                "[webhooks] Tunnel {} not found for unregister (skill '{}')",
                tunnel_uuid, skill_id
            );
        }

        drop(routes);
        self.publish_event("registration_changed", None, Some(tunnel_uuid.to_string()));
        self.persist();

        publish_global(DomainEvent::WebhookUnregistered {
            tunnel_id: tunnel_uuid.to_string(),
            skill_id: skill_id.to_string(),
        });

        Ok(())
    }

    /// Remove all tunnel registrations for a skill (called on skill stop/crash).
    pub fn unregister_skill(&self, skill_id: &str) {
        let mut routes = match self.routes.write() {
            Ok(r) => r,
            Err(e) => {
                warn!("[webhooks] Failed to acquire write lock: {}", e);
                return;
            }
        };

        let removed_tunnels: Vec<String> = routes
            .iter()
            .filter(|(_, reg)| reg.skill_id == skill_id)
            .map(|(uuid, _)| uuid.clone())
            .collect();

        routes.retain(|_, reg| reg.skill_id != skill_id);

        if !removed_tunnels.is_empty() {
            debug!(
                "[webhooks] Unregistered {} tunnel(s) for skill '{}'",
                removed_tunnels.len(),
                skill_id
            );
            drop(routes);
            self.publish_event("registration_changed", None, None);
            self.persist();

            for tunnel_id in removed_tunnels {
                publish_global(DomainEvent::WebhookUnregistered {
                    tunnel_id,
                    skill_id: skill_id.to_string(),
                });
            }
        }
    }

    /// Look up which skill owns a tunnel UUID.
    pub fn route(&self, tunnel_uuid: &str) -> Option<String> {
        self.routes
            .read()
            .ok()?
            .get(tunnel_uuid)
            .filter(|registration| registration.target_kind == "skill")
            .map(|r| r.skill_id.clone())
    }

    /// Look up the full registration for a tunnel UUID.
    pub fn registration(&self, tunnel_uuid: &str) -> Option<TunnelRegistration> {
        self.routes.read().ok()?.get(tunnel_uuid).cloned()
    }

    /// List tunnels owned by a specific skill (for the skill JS API).
    pub fn list_for_skill(&self, skill_id: &str) -> Vec<TunnelRegistration> {
        self.routes
            .read()
            .map(|routes| {
                routes
                    .values()
                    .filter(|r| r.skill_id == skill_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List all tunnel registrations (for the frontend admin UI).
    pub fn list_all(&self) -> Vec<TunnelRegistration> {
        self.routes
            .read()
            .map(|routes| routes.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Record an incoming webhook request before routing completes.
    pub fn record_request(&self, request: &WebhookRequest, skill_id: Option<String>) {
        let now = now_ms();
        let correlation_id = request.correlation_id.clone();
        let tunnel_uuid = request.tunnel_uuid.clone();
        let entry = WebhookDebugLogEntry {
            correlation_id: correlation_id.clone(),
            tunnel_id: request.tunnel_id.clone(),
            tunnel_uuid: tunnel_uuid.clone(),
            tunnel_name: request.tunnel_name.clone(),
            method: request.method.clone(),
            path: request.path.clone(),
            skill_id,
            status_code: None,
            timestamp: now,
            updated_at: now,
            request_headers: request.headers.clone(),
            request_query: request.query.clone(),
            request_body: request.body.clone(),
            response_headers: HashMap::new(),
            response_body: String::new(),
            stage: "received".to_string(),
            error_message: None,
            raw_payload: None,
        };

        self.upsert_log(entry);
        self.publish_event("log_updated", Some(correlation_id), Some(tunnel_uuid));
    }

    /// Record a malformed webhook request that could not be fully parsed.
    pub fn record_parse_error(
        &self,
        correlation_id: String,
        tunnel_uuid: Option<String>,
        method: Option<String>,
        path: Option<String>,
        raw_payload: serde_json::Value,
        error_message: String,
    ) {
        let now = now_ms();
        let entry = WebhookDebugLogEntry {
            correlation_id: correlation_id.clone(),
            tunnel_id: String::new(),
            tunnel_uuid: tunnel_uuid.clone().unwrap_or_default(),
            tunnel_name: "unknown".to_string(),
            method: method.unwrap_or_else(|| "UNKNOWN".to_string()),
            path: path.unwrap_or_else(|| "/".to_string()),
            skill_id: None,
            status_code: Some(400),
            timestamp: now,
            updated_at: now,
            request_headers: HashMap::new(),
            request_query: HashMap::new(),
            request_body: String::new(),
            response_headers: HashMap::new(),
            response_body: String::new(),
            stage: "parse_error".to_string(),
            error_message: Some(error_message),
            raw_payload: Some(raw_payload),
        };

        self.upsert_log(entry);
        self.publish_event("log_updated", Some(correlation_id), tunnel_uuid);
    }

    /// Record the final response for a webhook request.
    pub fn record_response(
        &self,
        request: &WebhookRequest,
        response: &WebhookResponseData,
        skill_id: Option<String>,
        error_message: Option<String>,
    ) {
        let now = now_ms();
        let correlation_id = request.correlation_id.clone();
        let tunnel_uuid = request.tunnel_uuid.clone();

        if let Ok(mut logs) = self.debug_logs.write() {
            if let Some(existing) = logs
                .iter_mut()
                .find(|entry| entry.correlation_id == request.correlation_id)
            {
                existing.skill_id = skill_id.clone().or_else(|| existing.skill_id.clone());
                existing.status_code = Some(response.status_code);
                existing.updated_at = now;
                existing.response_headers = response.headers.clone();
                existing.response_body = response.body.clone();
                existing.stage = if error_message.is_some() {
                    "error".to_string()
                } else {
                    "completed".to_string()
                };
                existing.error_message = error_message.clone();
            } else {
                logs.push_front(WebhookDebugLogEntry {
                    correlation_id: request.correlation_id.clone(),
                    tunnel_id: request.tunnel_id.clone(),
                    tunnel_uuid: request.tunnel_uuid.clone(),
                    tunnel_name: request.tunnel_name.clone(),
                    method: request.method.clone(),
                    path: request.path.clone(),
                    skill_id,
                    status_code: Some(response.status_code),
                    timestamp: now,
                    updated_at: now,
                    request_headers: request.headers.clone(),
                    request_query: request.query.clone(),
                    request_body: request.body.clone(),
                    response_headers: response.headers.clone(),
                    response_body: response.body.clone(),
                    stage: if error_message.is_some() {
                        "error".to_string()
                    } else {
                        "completed".to_string()
                    },
                    error_message,
                    raw_payload: None,
                });
                truncate_logs(&mut logs);
            }
        }

        self.publish_event("log_updated", Some(correlation_id), Some(tunnel_uuid));
    }

    /// List recent webhook logs, newest first.
    pub fn list_logs(&self, limit: Option<usize>) -> Vec<WebhookDebugLogEntry> {
        let limit = limit.unwrap_or(100).max(1);
        self.debug_logs
            .read()
            .map(|logs| logs.iter().take(limit).cloned().collect())
            .unwrap_or_default()
    }

    /// Clear all captured webhook logs. Returns the number removed.
    pub fn clear_logs(&self) -> usize {
        let cleared = self
            .debug_logs
            .write()
            .map(|mut logs| {
                let len = logs.len();
                logs.clear();
                len
            })
            .unwrap_or(0);

        if cleared > 0 {
            self.publish_event("logs_cleared", None, None);
        }

        cleared
    }

    pub fn subscribe_debug_events(&self) -> broadcast::Receiver<WebhookDebugEvent> {
        WEBHOOK_DEBUG_EVENTS.subscribe()
    }

    /// Persist current routes to disk.
    fn persist(&self) {
        let Some(ref path) = self.persist_path else {
            return;
        };

        // Clone routes under the lock, then release before doing I/O.
        let persisted = {
            let routes = match self.routes.read() {
                Ok(r) => r,
                Err(_) => return,
            };
            PersistedRoutes {
                registrations: routes.values().cloned().collect(),
            }
        };

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        match serde_json::to_string_pretty(&persisted) {
            Ok(json) => {
                if let Err(e) = std::fs::write(path, json) {
                    warn!("[webhooks] Failed to persist routes to {:?}: {}", path, e);
                }
            }
            Err(e) => {
                warn!("[webhooks] Failed to serialize routes: {}", e);
            }
        }
    }

    fn upsert_log(&self, entry: WebhookDebugLogEntry) {
        if let Ok(mut logs) = self.debug_logs.write() {
            if let Some(existing) = logs
                .iter_mut()
                .find(|current| current.correlation_id == entry.correlation_id)
            {
                *existing = entry;
            } else {
                logs.push_front(entry);
                truncate_logs(&mut logs);
            }
        }
    }

    fn publish_event(
        &self,
        event_type: &str,
        correlation_id: Option<String>,
        tunnel_uuid: Option<String>,
    ) {
        let _ = WEBHOOK_DEBUG_EVENTS.send(WebhookDebugEvent {
            event_type: event_type.to_string(),
            timestamp: now_ms(),
            correlation_id,
            tunnel_uuid,
        });
    }
}

fn truncate_logs(logs: &mut VecDeque<WebhookDebugLogEntry>) {
    while logs.len() > MAX_DEBUG_LOG_ENTRIES {
        logs.pop_back();
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
