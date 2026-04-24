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
            None,
        )
    }

    /// Register a built-in echo webhook target for ad-hoc testing.
    pub fn register_echo(
        &self,
        tunnel_uuid: &str,
        tunnel_name: Option<String>,
        backend_tunnel_id: Option<String>,
    ) -> Result<(), String> {
        self.register_target(
            tunnel_uuid,
            "echo",
            "echo",
            tunnel_name,
            backend_tunnel_id,
            None,
        )
    }

    /// Register an agent-backed webhook tunnel.
    ///
    /// Requests arriving on this tunnel are routed into the triage
    /// pipeline rather than the skill runtime. `agent_id` is stored
    /// for observability and rebind validation; the triage evaluator
    /// currently selects the target agent dynamically regardless of
    /// this value.
    pub fn register_agent(
        &self,
        tunnel_uuid: &str,
        agent_id: Option<String>,
        tunnel_name: Option<String>,
        backend_tunnel_id: Option<String>,
    ) -> Result<(), String> {
        self.register_target(
            tunnel_uuid,
            "agent",
            "agent",
            tunnel_name,
            backend_tunnel_id,
            agent_id,
        )
    }

    fn register_target(
        &self,
        tunnel_uuid: &str,
        target_kind: &str,
        skill_id: &str,
        tunnel_name: Option<String>,
        backend_tunnel_id: Option<String>,
        agent_id: Option<String>,
    ) -> Result<(), String> {
        let mut routes = self.routes.write().map_err(|e| e.to_string())?;

        if let Some(existing) = routes.get(tunnel_uuid) {
            if existing.skill_id != skill_id || existing.target_kind != target_kind {
                return Err(format!(
                    "Tunnel {} is already owned by {} '{}'; {} '{}' cannot register it",
                    tunnel_uuid, existing.target_kind, existing.skill_id, target_kind, skill_id
                ));
            }
            // Prevent silent agent_id rebinding on agent tunnels.
            if target_kind == "agent" && existing.agent_id.as_deref() != agent_id.as_deref() {
                tracing::warn!(
                    tunnel = %tunnel_uuid,
                    existing_agent = ?existing.agent_id,
                    requested_agent = ?agent_id,
                    "[webhooks] rejecting agent tunnel rebind"
                );
                return Err(format!(
                    "Tunnel {} is already bound to agent {:?}; cannot rebind to {:?}",
                    tunnel_uuid, existing.agent_id, agent_id
                ));
            }
        }

        debug!(
            "[webhooks] Registering tunnel {} → {} '{}' (agent={:?})",
            tunnel_uuid, target_kind, skill_id, agent_id,
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
                agent_id,
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_register_and_route() {
        let router = WebhookRouter::new(None);
        router
            .register("uuid-1", "gmail", Some("Gmail Webhook".into()), None)
            .unwrap();

        assert_eq!(router.route("uuid-1"), Some("gmail".to_string()));
        assert_eq!(router.route("uuid-nonexistent"), None);
    }

    #[test]
    fn test_ownership_enforcement() {
        let router = WebhookRouter::new(None);
        router
            .register("uuid-1", "gmail", Some("Gmail".into()), None)
            .unwrap();

        // Another skill cannot register the same tunnel
        let result = router.register("uuid-1", "notion", Some("Notion".into()), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already owned"));

        // Same skill can re-register (update)
        router
            .register("uuid-1", "gmail", Some("Gmail Updated".into()), None)
            .unwrap();
    }

    #[test]
    fn test_unregister_ownership() {
        let router = WebhookRouter::new(None);
        router.register("uuid-1", "gmail", None, None).unwrap();

        // Another skill cannot unregister
        let result = router.unregister("uuid-1", "notion");
        assert!(result.is_err());

        // Owner can unregister
        router.unregister("uuid-1", "gmail").unwrap();
        assert_eq!(router.route("uuid-1"), None);
    }

    #[test]
    fn test_unregister_skill() {
        let router = WebhookRouter::new(None);
        router.register("uuid-1", "gmail", None, None).unwrap();
        router.register("uuid-2", "gmail", None, None).unwrap();
        router.register("uuid-3", "notion", None, None).unwrap();

        router.unregister_skill("gmail");

        assert_eq!(router.route("uuid-1"), None);
        assert_eq!(router.route("uuid-2"), None);
        assert_eq!(router.route("uuid-3"), Some("notion".to_string()));
    }

    #[test]
    fn test_list_for_skill() {
        let router = WebhookRouter::new(None);
        router.register("uuid-1", "gmail", None, None).unwrap();
        router.register("uuid-2", "notion", None, None).unwrap();
        router.register("uuid-3", "gmail", None, None).unwrap();

        let gmail_tunnels = router.list_for_skill("gmail");
        assert_eq!(gmail_tunnels.len(), 2);
        assert!(gmail_tunnels.iter().all(|t| t.skill_id == "gmail"));

        let notion_tunnels = router.list_for_skill("notion");
        assert_eq!(notion_tunnels.len(), 1);

        let empty = router.list_for_skill("nonexistent");
        assert!(empty.is_empty());
    }

    #[test]
    fn test_record_request_and_response() {
        let router = WebhookRouter::new(None);
        let request = WebhookRequest {
            correlation_id: "corr-1".to_string(),
            tunnel_id: "tunnel-id-1".to_string(),
            tunnel_uuid: "uuid-1".to_string(),
            tunnel_name: "Inbox".to_string(),
            method: "POST".to_string(),
            path: "/hooks/test".to_string(),
            headers: HashMap::from([(String::from("x-test"), json!("1"))]),
            query: HashMap::from([(String::from("hello"), String::from("world"))]),
            body: "aGVsbG8=".to_string(),
        };
        let response = WebhookResponseData {
            correlation_id: "corr-1".to_string(),
            status_code: 204,
            headers: HashMap::from([(String::from("x-ok"), String::from("yes"))]),
            body: String::new(),
        };

        router.record_request(&request, Some("gmail".to_string()));
        router.record_response(&request, &response, Some("gmail".to_string()), None);

        let logs = router.list_logs(Some(10));
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].correlation_id, "corr-1");
        assert_eq!(logs[0].status_code, Some(204));
        assert_eq!(logs[0].skill_id.as_deref(), Some("gmail"));
        assert_eq!(logs[0].stage, "completed");
    }

    #[test]
    fn test_clear_logs() {
        let router = WebhookRouter::new(None);
        router.record_parse_error(
            "corr-2".to_string(),
            Some("uuid-2".to_string()),
            Some("POST".to_string()),
            Some("/broken".to_string()),
            json!({ "broken": true }),
            "bad payload".to_string(),
        );

        assert_eq!(router.list_logs(Some(10)).len(), 1);
        assert_eq!(router.clear_logs(), 1);
        assert!(router.list_logs(Some(10)).is_empty());
    }

    #[test]
    fn register_echo_and_route_returns_none_for_echo_targets() {
        let router = WebhookRouter::new(None);
        router
            .register_echo("uuid-echo", Some("Test Echo".into()), None)
            .unwrap();
        // Echo targets are target_kind="echo", route() only returns "skill" targets
        assert_eq!(router.route("uuid-echo"), None);
    }

    #[test]
    fn registration_returns_full_tunnel_info() {
        let router = WebhookRouter::new(None);
        router
            .register(
                "uuid-1",
                "gmail",
                Some("My Tunnel".into()),
                Some("bt-1".into()),
            )
            .unwrap();
        let reg = router.registration("uuid-1").unwrap();
        assert_eq!(reg.tunnel_uuid, "uuid-1");
        assert_eq!(reg.skill_id, "gmail");
        assert_eq!(reg.tunnel_name.as_deref(), Some("My Tunnel"));
        assert_eq!(reg.backend_tunnel_id.as_deref(), Some("bt-1"));
    }

    #[test]
    fn registration_returns_none_for_missing_uuid() {
        let router = WebhookRouter::new(None);
        assert!(router.registration("no-such").is_none());
    }

    #[test]
    fn list_all_returns_all_registrations() {
        let router = WebhookRouter::new(None);
        router.register("u1", "s1", None, None).unwrap();
        router.register("u2", "s2", None, None).unwrap();
        let all = router.list_all();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn list_logs_respects_limit() {
        let router = WebhookRouter::new(None);
        for i in 0..5 {
            router.record_parse_error(
                format!("corr-{i}"),
                None,
                None,
                None,
                json!({}),
                "error".into(),
            );
        }
        let logs = router.list_logs(Some(3));
        assert_eq!(logs.len(), 3);
    }

    #[test]
    fn list_logs_default_limit() {
        let router = WebhookRouter::new(None);
        for i in 0..5 {
            router.record_parse_error(
                format!("corr-{i}"),
                None,
                None,
                None,
                json!({}),
                "err".into(),
            );
        }
        let logs = router.list_logs(None);
        assert_eq!(logs.len(), 5); // less than default limit of 100
    }

    #[test]
    fn record_response_without_prior_request_creates_new_entry() {
        let router = WebhookRouter::new(None);
        let request = WebhookRequest {
            correlation_id: "corr-new".into(),
            tunnel_id: "tid".into(),
            tunnel_uuid: "uuid-new".into(),
            tunnel_name: "Test".into(),
            method: "POST".into(),
            path: "/test".into(),
            headers: HashMap::new(),
            query: HashMap::new(),
            body: String::new(),
        };
        let response = WebhookResponseData {
            correlation_id: "corr-new".into(),
            status_code: 200,
            headers: HashMap::new(),
            body: "ok".into(),
        };
        // No prior record_request — should still create a log entry
        router.record_response(&request, &response, None, None);
        let logs = router.list_logs(Some(10));
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].stage, "completed");
    }

    #[test]
    fn record_response_with_error_sets_error_stage() {
        let router = WebhookRouter::new(None);
        let request = WebhookRequest {
            correlation_id: "corr-err".into(),
            tunnel_id: "tid".into(),
            tunnel_uuid: "uuid-err".into(),
            tunnel_name: "Test".into(),
            method: "POST".into(),
            path: "/test".into(),
            headers: HashMap::new(),
            query: HashMap::new(),
            body: String::new(),
        };
        let response = WebhookResponseData {
            correlation_id: "corr-err".into(),
            status_code: 500,
            headers: HashMap::new(),
            body: String::new(),
        };
        router.record_request(&request, None);
        router.record_response(&request, &response, None, Some("handler crashed".into()));
        let logs = router.list_logs(Some(10));
        assert_eq!(logs[0].stage, "error");
        assert_eq!(logs[0].error_message.as_deref(), Some("handler crashed"));
    }

    #[test]
    fn clear_logs_returns_zero_when_empty() {
        let router = WebhookRouter::new(None);
        assert_eq!(router.clear_logs(), 0);
    }

    #[test]
    fn subscribe_debug_events_does_not_panic() {
        let router = WebhookRouter::new(None);
        let _rx = router.subscribe_debug_events();
    }

    #[test]
    fn persist_and_load_roundtrip() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        let router = WebhookRouter::new(Some(path.clone()));
        router
            .register("uuid-p1", "skill-a", Some("Tunnel A".into()), None)
            .unwrap();
        router
            .register("uuid-p2", "skill-b", None, Some("bt-2".into()))
            .unwrap();

        // Load from disk
        let router2 = WebhookRouter::new(Some(path));
        assert_eq!(router2.list_all().len(), 2);
        assert!(router2.registration("uuid-p1").is_some());
        assert!(router2.registration("uuid-p2").is_some());
    }

    #[test]
    fn unregister_nonexistent_tunnel_is_noop() {
        let router = WebhookRouter::new(None);
        // Should not error even though tunnel doesn't exist
        router.unregister("no-such", "any-skill").unwrap();
    }

    #[test]
    fn unregister_skill_with_no_tunnels_is_noop() {
        let router = WebhookRouter::new(None);
        router.register("u1", "other", None, None).unwrap();
        router.unregister_skill("nonexistent");
        assert_eq!(router.list_all().len(), 1);
    }

    #[test]
    fn record_parse_error_creates_entry_with_parse_error_stage() {
        let router = WebhookRouter::new(None);
        router.record_parse_error(
            "corr-p".into(),
            Some("uuid-p".into()),
            Some("GET".into()),
            Some("/bad".into()),
            json!({"raw": true}),
            "malformed body".into(),
        );
        let logs = router.list_logs(Some(1));
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].stage, "parse_error");
        assert_eq!(logs[0].status_code, Some(400));
        assert_eq!(logs[0].error_message.as_deref(), Some("malformed body"));
    }

    #[test]
    fn truncate_logs_respects_max() {
        let router = WebhookRouter::new(None);
        for i in 0..(MAX_DEBUG_LOG_ENTRIES + 10) {
            router.record_parse_error(format!("c-{i}"), None, None, None, json!({}), "e".into());
        }
        let logs = router.list_logs(Some(MAX_DEBUG_LOG_ENTRIES + 100));
        assert!(logs.len() <= MAX_DEBUG_LOG_ENTRIES);
    }

    #[test]
    fn register_agent_persists_agent_id_and_name() {
        let router = WebhookRouter::new(None);
        router
            .register_agent(
                "uuid-a1",
                Some("agent-42".into()),
                Some("My Agent".into()),
                None,
            )
            .unwrap();

        let reg = router.registration("uuid-a1").unwrap();
        assert_eq!(reg.target_kind, "agent");
        assert_eq!(reg.agent_id.as_deref(), Some("agent-42"));
        assert_eq!(reg.tunnel_name.as_deref(), Some("My Agent"));
    }

    #[test]
    fn register_agent_same_id_succeeds() {
        let router = WebhookRouter::new(None);
        router
            .register_agent("uuid-a2", Some("agent-1".into()), None, None)
            .unwrap();
        // Re-register with the same agent_id should succeed.
        router
            .register_agent(
                "uuid-a2",
                Some("agent-1".into()),
                Some("Updated".into()),
                None,
            )
            .unwrap();

        let reg = router.registration("uuid-a2").unwrap();
        assert_eq!(reg.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(reg.tunnel_name.as_deref(), Some("Updated"));
    }

    #[test]
    fn register_agent_rejects_different_agent_id() {
        let router = WebhookRouter::new(None);
        router
            .register_agent("uuid-a3", Some("agent-A".into()), None, None)
            .unwrap();

        let err = router
            .register_agent("uuid-a3", Some("agent-B".into()), None, None)
            .unwrap_err();
        assert!(err.contains("already bound"));

        // Original agent_id is preserved.
        let reg = router.registration("uuid-a3").unwrap();
        assert_eq!(reg.agent_id.as_deref(), Some("agent-A"));
    }
}
