//! Shared types, state structs, and helpers for QuickJS operations.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

// ============================================================================
// Timer State
// ============================================================================

/// Represents a single timer (setTimeout or setInterval) in the JS environment.
#[derive(Debug)]
pub struct TimerEntry {
    /// The absolute time when this timer should trigger.
    pub deadline: Instant,
    /// The delay in milliseconds between executions (used for intervals).
    pub delay_ms: u32,
    /// Whether this is a recurring interval timer.
    pub is_interval: bool,
}

/// Tracks all active timers for a single QuickJS instance.
#[derive(Debug, Default)]
pub struct TimerState {
    /// Active timers keyed by their unique numeric ID.
    pub timers: HashMap<u32, TimerEntry>,
}

impl TimerState {
    /// Polls all timers and returns a list of IDs that have reached their deadline.
    /// Recurring interval timers have their deadlines updated for the next execution.
    pub fn poll_ready(&mut self) -> Vec<u32> {
        let now = Instant::now();
        let mut ready = Vec::new();
        let mut to_remove = Vec::new();

        // Identify timers that have expired
        for (&id, entry) in &self.timers {
            if now >= entry.deadline {
                ready.push(id);
                if !entry.is_interval {
                    to_remove.push(id);
                }
            }
        }

        // Clean up one-shot timers
        for id in to_remove {
            self.timers.remove(&id);
        }

        // Reschedule recurring intervals
        for &id in &ready {
            if let Some(entry) = self.timers.get_mut(&id) {
                if entry.is_interval {
                    entry.deadline = now + Duration::from_millis(entry.delay_ms as u64);
                }
            }
        }

        ready
    }

    /// Returns the duration until the next timer is scheduled to trigger.
    /// Returns `None` if no timers are active.
    pub fn time_until_next(&self) -> Option<Duration> {
        let now = Instant::now();
        self.timers
            .values()
            .map(|e| e.deadline.saturating_duration_since(now))
            .min()
    }
}

/// Helper function to poll timers and calculate the wait time until the next event.
pub fn poll_timers(timer_state: &RwLock<TimerState>) -> (Vec<u32>, Option<Duration>) {
    let mut ts = timer_state.write();
    let ready = ts.poll_ready();
    let next = ts.time_until_next();
    (ready, next)
}

// ============================================================================
// Skill Context
// ============================================================================

/// Holds environment-level context for a specific skill.
#[derive(Clone)]
pub struct SkillContext {
    /// Unique identifier for the skill.
    pub skill_id: String,
    /// Path to the directory where the skill can store persistent data.
    pub data_dir: PathBuf,
    /// Optional client for interacting with the OpenHuman memory system.
    pub memory_client: Option<crate::openhuman::memory::MemoryClientRef>,
    /// Optional router for handling incoming webhooks for this skill.
    pub webhook_router: Option<std::sync::Arc<crate::openhuman::webhooks::WebhookRouter>>,
}

// ============================================================================
// Skill State (shared published state)
// ============================================================================

/// Represents the observable/shared state of a skill.
/// This state is typically synced between the Rust backend and the JS environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillState {
    /// The actual data stored in the state, represented as a JSON map.
    #[serde(flatten)]
    pub data: serde_json::Map<String, serde_json::Value>,
    /// Set to true when data is modified; the event loop clears it after syncing.
    #[serde(skip)]
    pub dirty: bool,
}

impl Default for SkillState {
    fn default() -> Self {
        Self {
            data: serde_json::Map::new(),
            dirty: false,
        }
    }
}

// ============================================================================
// WebSocket State (placeholder)
// ============================================================================

/// Represents an active WebSocket connection.
#[derive(Debug)]
pub struct WebSocketConnection {
    /// The remote URL of the WebSocket.
    pub url: String,
}

/// Tracks all active WebSocket connections for a QuickJS instance.
#[derive(Debug, Default)]
pub struct WebSocketState {
    /// Active connections keyed by their unique numeric ID.
    pub connections: HashMap<u32, WebSocketConnection>,
    /// The next ID to assign to a new connection.
    pub next_id: u32,
}

// ============================================================================
// Constants & Helpers
// ============================================================================

/// List of environment variables that are allowed to be accessed from within the JS environment.
pub const ALLOWED_ENV_VARS: &[&str] = &["VITE_BACKEND_URL", "BACKEND_URL", "JWT_TOKEN", "NODE_ENV"];

/// Sanitize error message for use with QuickJS/rquickjs.
///
/// This avoids characters that might trigger rquickjs internal errors or formatting issues
/// when creating a JS exception from a Rust error.
fn sanitize_error_message(msg: &str) -> String {
    msg.chars()
        .map(|c| {
            if c == ',' || c == '-' {
                ' '
            } else if c.is_ascii() && !c.is_ascii_control() {
                c
            } else if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                '?'
            }
        })
        .collect()
}

/// Creates a `rquickjs::Error` with a sanitized message.
pub fn js_err(msg: impl AsRef<str>) -> rquickjs::Error {
    let sanitized = sanitize_error_message(msg.as_ref());
    rquickjs::Error::new_from_js_message("skill", "RuntimeError", sanitized)
}
