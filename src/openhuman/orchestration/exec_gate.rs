//! Device-authoritative trust gate for local-execution device tools.
//!
//! The hosted brain runs in the cloud and can push `orch:tool_call` frames for
//! device-declared tools. Some of those tools (`run_local_agent` → the local
//! `code_executor` / workspace workers) execute code and read files on the
//! user's machine. Those must run **only** for a Master-chat cycle (the human
//! talking to their own OpenHuman), never for an A2A cycle driven by another
//! agent's DM — otherwise a prompt-injected reasoning turn could induce local
//! code execution / file exfiltration (confused deputy).
//!
//! The gate is **device-authoritative**: it does not trust any origin the
//! backend asserts in the tool-call frame (an injected brain could lie). When
//! the device forwards an event to the hosted brain (`POST /events`) it already
//! knows the cycle's counterpart, and the backend returns the `cycleId` for
//! that trigger. We record `cycleId -> counterpart` from *our own* forward, so
//! at tool-call time we resolve the origin from a fact we established, not one
//! the backend supplied. Unknown cycles fail closed (denied).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use super::types::LOCAL_MASTER_AGENT;

/// Bound on tracked cycles — cycles are short-lived, so a small ring is plenty
/// and keeps this from growing unbounded over a long session.
const MAX_TRACKED: usize = 512;

/// The counterpart + session a forwarded cycle belongs to (device-recorded).
#[derive(Clone)]
struct CycleTarget {
    counterpart: String,
    session_id: String,
}

struct CycleOrigins {
    target_by_cycle: HashMap<String, CycleTarget>,
    /// FIFO of cycle ids for bounded eviction of the oldest entries.
    order: VecDeque<String>,
}

static CYCLE_ORIGINS: Mutex<Option<CycleOrigins>> = Mutex::new(None);

/// Record the counterpart + session a forwarded cycle belongs to. Called at
/// forward time (`ingest` / master-send → `cloud::push_event`) with the
/// `cycleId` the backend returned and the counterpart/session *we* addressed the
/// event to. This is the device-authoritative fact the gate resolves against.
pub fn record_cycle_origin(cycle_id: &str, counterpart: &str, session_id: &str) {
    if cycle_id.is_empty() {
        return;
    }
    let mut guard = CYCLE_ORIGINS.lock().unwrap_or_else(|p| p.into_inner());
    let store = guard.get_or_insert_with(|| CycleOrigins {
        target_by_cycle: HashMap::new(),
        order: VecDeque::new(),
    });
    let target = CycleTarget {
        counterpart: counterpart.to_string(),
        session_id: session_id.to_string(),
    };
    if store
        .target_by_cycle
        .insert(cycle_id.to_string(), target)
        .is_none()
    {
        store.order.push_back(cycle_id.to_string());
        while store.order.len() > MAX_TRACKED {
            if let Some(old) = store.order.pop_front() {
                store.target_by_cycle.remove(&old);
            }
        }
    }
}

/// True only when `cycle_id` was recorded as a **local Master-chat** cycle
/// (counterpart == [`LOCAL_MASTER_AGENT`]). Unknown or A2A cycles → false
/// (fail closed). This is the authorization for local-execution device tools.
pub fn cycle_is_master(cycle_id: &str) -> bool {
    let guard = CYCLE_ORIGINS.lock().unwrap_or_else(|p| p.into_inner());
    guard
        .as_ref()
        .and_then(|s| s.target_by_cycle.get(cycle_id))
        .map(|t| t.counterpart == LOCAL_MASTER_AGENT)
        .unwrap_or(false)
}

/// The `(counterpart, session_id)` a cycle targets, for routing a local
/// sub-agent's completion back to the right session. `None` for unknown cycles.
pub fn cycle_target(cycle_id: &str) -> Option<(String, String)> {
    let guard = CYCLE_ORIGINS.lock().unwrap_or_else(|p| p.into_inner());
    guard
        .as_ref()
        .and_then(|s| s.target_by_cycle.get(cycle_id))
        .map(|t| (t.counterpart.clone(), t.session_id.clone()))
}

/// Device tools that touch local code / files / shell. Denied for any non-Master
/// cycle. Kept as an explicit allowlist so adding a tool to the manifest can't
/// silently escape the gate.
pub fn is_local_execution_tool(name: &str) -> bool {
    matches!(name, "run_local_agent")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_cycle_is_not_master() {
        assert!(!cycle_is_master("never-seen"));
    }

    #[test]
    fn master_cycle_is_allowed_a2a_is_denied() {
        record_cycle_origin("cyc-master", LOCAL_MASTER_AGENT, "master");
        record_cycle_origin("cyc-peer", "8xPeerBase58Addr", "sess-1");
        assert!(cycle_is_master("cyc-master"));
        assert!(!cycle_is_master("cyc-peer"));
        assert_eq!(
            cycle_target("cyc-master"),
            Some((LOCAL_MASTER_AGENT.to_string(), "master".to_string()))
        );
    }

    #[test]
    fn empty_cycle_id_is_ignored_and_denied() {
        record_cycle_origin("", LOCAL_MASTER_AGENT, "master");
        assert!(!cycle_is_master(""));
        assert_eq!(cycle_target(""), None);
    }

    #[test]
    fn local_execution_tools_are_gated() {
        assert!(is_local_execution_tool("run_local_agent"));
        assert!(!is_local_execution_tool("device_status"));
    }
}
