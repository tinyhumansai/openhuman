use chrono::Utc;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Instant;

#[derive(Debug, Clone, Serialize)]
pub struct ComponentHealth {
    pub status: String,
    pub updated_at: String,
    pub last_ok: Option<String>,
    pub last_error: Option<String>,
    pub restart_count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthSnapshot {
    pub pid: u32,
    pub updated_at: String,
    pub uptime_seconds: u64,
    pub components: BTreeMap<String, ComponentHealth>,
}

struct HealthRegistry {
    started_at: Instant,
    components: Mutex<BTreeMap<String, ComponentHealth>>,
}

static REGISTRY: OnceLock<HealthRegistry> = OnceLock::new();

fn registry() -> &'static HealthRegistry {
    REGISTRY.get_or_init(|| HealthRegistry {
        started_at: Instant::now(),
        components: Mutex::new(BTreeMap::new()),
    })
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn upsert_component<F>(component: &str, update: F)
where
    F: FnOnce(&mut ComponentHealth),
{
    let mut map = registry().components.lock();
    let now = now_rfc3339();
    let entry = map
        .entry(component.to_string())
        .or_insert_with(|| ComponentHealth {
            status: "starting".into(),
            updated_at: now.clone(),
            last_ok: None,
            last_error: None,
            restart_count: 0,
        });
    update(entry);
    entry.updated_at = now;
}

pub fn mark_component_ok(component: &str) {
    log::debug!("[openhuman:health] Component '{}' marked OK", component);
    upsert_component(component, |entry| {
        entry.status = "ok".into();
        entry.last_ok = Some(now_rfc3339());
        entry.last_error = None;
    });
}

#[allow(clippy::needless_pass_by_value)]
pub fn mark_component_error(component: &str, error: impl ToString) {
    let err = error.to_string();
    log::warn!(
        "[openhuman:health] Component '{}' error: {}",
        component,
        err
    );
    upsert_component(component, move |entry| {
        entry.status = "error".into();
        entry.last_error = Some(err);
    });
}

pub fn bump_component_restart(component: &str) {
    log::info!("[openhuman:health] Component '{}' restarting", component);
    upsert_component(component, |entry| {
        entry.restart_count = entry.restart_count.saturating_add(1);
    });
}

pub fn snapshot() -> HealthSnapshot {
    let components = registry().components.lock().clone();

    HealthSnapshot {
        pid: std::process::id(),
        updated_at: now_rfc3339(),
        uptime_seconds: registry().started_at.elapsed().as_secs(),
        components,
    }
}

pub fn snapshot_json() -> serde_json::Value {
    serde_json::to_value(snapshot()).unwrap_or_else(|_| {
        serde_json::json!({
            "status": "error",
            "message": "failed to serialize health snapshot"
        })
    })
}

/// Components whose sustained failure means the whole container should be
/// recycled — and the **only** ones whose unhealth makes `/health` return 503.
///
/// Everything else is a degradable background service whose failure must NOT
/// flip the container `unhealthy` (#3312): a single cron-job timeout marked the
/// `scheduler` component `error` and 503'd the container for 7h43m even though
/// the core RPC was serving fine the whole time. `scheduler`, `channels`, and
/// `update_checker` are therefore intentionally **non-critical**.
///
/// - `core` — the core process / RPC serving capability itself.
/// - `memory_tree_db` — the memory database. Its health signal is a *debounced*
///   circuit breaker that only trips after several consecutive schema-init
///   failures (a genuine, restart-worthy data-layer fault), so unlike the
///   scheduler case it does not false-trip on a transient blip.
///
/// New components default to **non-critical**: add a name here deliberately when
/// its failure should recycle the container.
const CRITICAL_COMPONENTS: &[&str] = &["core", "memory_tree_db"];

/// Whether `name` is a critical component (see [`CRITICAL_COMPONENTS`]).
pub fn is_critical_component(name: &str) -> bool {
    CRITICAL_COMPONENTS.contains(&name)
}

/// A component status counts as healthy for liveness purposes when it is `ok`
/// or still `starting` (boot grace — a component that hasn't reported yet must
/// not 503 the container).
fn is_healthy_status(status: &str) -> bool {
    status == "ok" || status == "starting"
}

/// Liveness/readiness verdict derived from a [`HealthSnapshot`]. Pure function
/// of the snapshot so it is unit-testable without the global registry.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HealthVerdict {
    /// True when no *critical* component is unhealthy → `/health` returns 200.
    pub healthy: bool,
    /// True when at least one *non-critical* component is unhealthy. The
    /// container stays live (200) but is degraded — surfaced for readiness /
    /// observability, not for the liveness 503.
    pub degraded: bool,
    /// Names of unhealthy critical components — these drive the 503.
    pub critical_unhealthy: Vec<String>,
    /// Names of unhealthy non-critical components (informational).
    pub degraded_components: Vec<String>,
}

/// Classify a snapshot into a [`HealthVerdict`]: a single degraded background
/// component no longer makes the whole container unhealthy — only an unhealthy
/// *critical* component does (#3312).
pub fn verdict(snapshot: &HealthSnapshot) -> HealthVerdict {
    let mut critical_unhealthy = Vec::new();
    let mut degraded_components = Vec::new();
    for (name, component) in &snapshot.components {
        if is_healthy_status(&component.status) {
            continue;
        }
        if is_critical_component(name) {
            critical_unhealthy.push(name.clone());
        } else {
            degraded_components.push(name.clone());
        }
    }
    HealthVerdict {
        healthy: critical_unhealthy.is_empty(),
        degraded: !degraded_components.is_empty(),
        critical_unhealthy,
        degraded_components,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_component(prefix: &str) -> String {
        format!("{prefix}-{}", uuid::Uuid::new_v4())
    }

    #[test]
    fn mark_component_ok_initializes_component_state() {
        let component = unique_component("health-ok");

        mark_component_ok(&component);

        let snapshot = snapshot();
        let entry = snapshot
            .components
            .get(&component)
            .expect("component should be present after mark_component_ok");

        assert_eq!(entry.status, "ok");
        assert!(entry.last_ok.is_some());
        assert!(entry.last_error.is_none());
    }

    #[test]
    fn mark_component_error_then_ok_clears_last_error() {
        let component = unique_component("health-error");

        mark_component_error(&component, "first failure");
        let error_snapshot = snapshot();
        let errored = error_snapshot
            .components
            .get(&component)
            .expect("component should exist after mark_component_error");
        assert_eq!(errored.status, "error");
        assert_eq!(errored.last_error.as_deref(), Some("first failure"));

        mark_component_ok(&component);
        let recovered_snapshot = snapshot();
        let recovered = recovered_snapshot
            .components
            .get(&component)
            .expect("component should exist after recovery");
        assert_eq!(recovered.status, "ok");
        assert!(recovered.last_error.is_none());
        assert!(recovered.last_ok.is_some());
    }

    #[test]
    fn bump_component_restart_increments_counter() {
        let component = unique_component("health-restart");

        bump_component_restart(&component);
        bump_component_restart(&component);

        let snapshot = snapshot();
        let entry = snapshot
            .components
            .get(&component)
            .expect("component should exist after restart bump");

        assert_eq!(entry.restart_count, 2);
    }

    #[test]
    fn snapshot_json_contains_registered_component_fields() {
        let component = unique_component("health-json");

        mark_component_ok(&component);

        let json = snapshot_json();
        let component_json = &json["components"][&component];

        assert_eq!(component_json["status"], "ok");
        assert!(component_json["updated_at"].as_str().is_some());
        assert!(component_json["last_ok"].as_str().is_some());
        assert!(json["uptime_seconds"].as_u64().is_some());
    }

    // ── Critical-component verdict (#3312) ────────────────────────────────

    fn component(status: &str) -> ComponentHealth {
        ComponentHealth {
            status: status.to_string(),
            updated_at: "2026-06-10T00:00:00Z".to_string(),
            last_ok: None,
            last_error: None,
            restart_count: 0,
        }
    }

    /// Build a synthetic snapshot from `(name, status)` pairs — lets the
    /// verdict be tested without mutating the process-global registry.
    fn snapshot_of(components: &[(&str, &str)]) -> HealthSnapshot {
        HealthSnapshot {
            pid: 1,
            updated_at: "2026-06-10T00:00:00Z".to_string(),
            uptime_seconds: 0,
            components: components
                .iter()
                .map(|(n, s)| ((*n).to_string(), component(s)))
                .collect(),
        }
    }

    #[test]
    fn critical_set_membership() {
        assert!(is_critical_component("core"));
        assert!(is_critical_component("memory_tree_db"));
        assert!(!is_critical_component("scheduler"));
        assert!(!is_critical_component("channels"));
        assert!(!is_critical_component("update_checker"));
    }

    #[test]
    fn all_ok_is_healthy_and_not_degraded() {
        let v = verdict(&snapshot_of(&[("core", "ok"), ("scheduler", "ok")]));
        assert!(v.healthy);
        assert!(!v.degraded);
        assert!(v.critical_unhealthy.is_empty());
        assert!(v.degraded_components.is_empty());
    }

    #[test]
    fn noncritical_failure_stays_healthy_but_degraded() {
        // The exact #3312 case: scheduler in error must NOT 503 the container.
        let v = verdict(&snapshot_of(&[("core", "ok"), ("scheduler", "error")]));
        assert!(v.healthy, "a degraded background service must not 503");
        assert!(v.degraded);
        assert_eq!(v.degraded_components, vec!["scheduler".to_string()]);
        assert!(v.critical_unhealthy.is_empty());
    }

    #[test]
    fn critical_failure_is_unhealthy() {
        let v = verdict(&snapshot_of(&[("memory_tree_db", "error")]));
        assert!(
            !v.healthy,
            "a critical component failure 503s the container"
        );
        assert_eq!(v.critical_unhealthy, vec!["memory_tree_db".to_string()]);
    }

    #[test]
    fn mixed_failures_report_both_buckets_and_503() {
        let v = verdict(&snapshot_of(&[
            ("core", "error"),
            ("scheduler", "error"),
            ("channels", "ok"),
        ]));
        assert!(!v.healthy);
        assert_eq!(v.critical_unhealthy, vec!["core".to_string()]);
        assert_eq!(v.degraded_components, vec!["scheduler".to_string()]);
    }

    #[test]
    fn starting_status_is_treated_as_healthy() {
        // Boot grace: a not-yet-reported component must not 503 nor degrade.
        let v = verdict(&snapshot_of(&[
            ("core", "starting"),
            ("scheduler", "starting"),
        ]));
        assert!(v.healthy);
        assert!(!v.degraded);
    }
}
