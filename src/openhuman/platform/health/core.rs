use chrono::Utc;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    pub status: String,
    pub updated_at: String,
    pub last_ok: Option<String>,
    pub last_error: Option<String>,
    pub restart_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[path = "core_tests.rs"]
mod tests;
