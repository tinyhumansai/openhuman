//! Webhook router — maps tunnel UUIDs to owning skills with isolation enforcement.

use super::types::TunnelRegistration;
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

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
                Err(_) => {
                    debug!("[webhooks] No persisted routes file at {:?}", path);
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        Self {
            routes: RwLock::new(routes),
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
        let mut routes = self.routes.write().map_err(|e| e.to_string())?;

        if let Some(existing) = routes.get(tunnel_uuid) {
            if existing.skill_id != skill_id {
                return Err(format!(
                    "Tunnel {} is already owned by skill '{}'; skill '{}' cannot register it",
                    tunnel_uuid, existing.skill_id, skill_id
                ));
            }
        }

        debug!(
            "[webhooks] Registering tunnel {} → skill '{}'",
            tunnel_uuid, skill_id
        );

        routes.insert(
            tunnel_uuid.to_string(),
            TunnelRegistration {
                tunnel_uuid: tunnel_uuid.to_string(),
                skill_id: skill_id.to_string(),
                tunnel_name,
                backend_tunnel_id,
            },
        );

        drop(routes);
        self.persist();
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
        self.persist();
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

        let before = routes.len();
        routes.retain(|_, reg| reg.skill_id != skill_id);
        let removed = before - routes.len();

        if removed > 0 {
            debug!(
                "[webhooks] Unregistered {} tunnel(s) for skill '{}'",
                removed, skill_id
            );
            drop(routes);
            self.persist();
        }
    }

    /// Look up which skill owns a tunnel UUID.
    pub fn route(&self, tunnel_uuid: &str) -> Option<String> {
        self.routes
            .read()
            .ok()?
            .get(tunnel_uuid)
            .map(|r| r.skill_id.clone())
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

    /// Persist current routes to disk.
    fn persist(&self) {
        let Some(ref path) = self.persist_path else {
            return;
        };

        let routes = match self.routes.read() {
            Ok(r) => r,
            Err(_) => return,
        };

        let persisted = PersistedRoutes {
            registrations: routes.values().cloned().collect(),
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        router
            .register("uuid-1", "gmail", None, None)
            .unwrap();

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
        router
            .register("uuid-1", "gmail", None, None)
            .unwrap();
        router
            .register("uuid-2", "gmail", None, None)
            .unwrap();
        router
            .register("uuid-3", "notion", None, None)
            .unwrap();

        router.unregister_skill("gmail");

        assert_eq!(router.route("uuid-1"), None);
        assert_eq!(router.route("uuid-2"), None);
        assert_eq!(router.route("uuid-3"), Some("notion".to_string()));
    }

    #[test]
    fn test_list_for_skill() {
        let router = WebhookRouter::new(None);
        router
            .register("uuid-1", "gmail", None, None)
            .unwrap();
        router
            .register("uuid-2", "notion", None, None)
            .unwrap();
        router
            .register("uuid-3", "gmail", None, None)
            .unwrap();

        let gmail_tunnels = router.list_for_skill("gmail");
        assert_eq!(gmail_tunnels.len(), 2);
        assert!(gmail_tunnels.iter().all(|t| t.skill_id == "gmail"));

        let notion_tunnels = router.list_for_skill("notion");
        assert_eq!(notion_tunnels.len(), 1);

        let empty = router.list_for_skill("nonexistent");
        assert!(empty.is_empty());
    }
}
