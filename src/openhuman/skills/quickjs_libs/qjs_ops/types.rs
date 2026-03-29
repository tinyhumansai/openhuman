//! Shared types, state structs, and helpers for QuickJS ops.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tauri::AppHandle;

// ============================================================================
// Timer State
// ============================================================================

#[derive(Debug)]
pub struct TimerEntry {
    pub deadline: Instant,
    pub delay_ms: u32,
    pub is_interval: bool,
}

#[derive(Debug, Default)]
pub struct TimerState {
    pub timers: HashMap<u32, TimerEntry>,
}

impl TimerState {
    pub fn poll_ready(&mut self) -> Vec<u32> {
        let now = Instant::now();
        let mut ready = Vec::new();
        let mut to_remove = Vec::new();

        for (&id, entry) in &self.timers {
            if now >= entry.deadline {
                ready.push(id);
                if !entry.is_interval {
                    to_remove.push(id);
                }
            }
        }

        for id in to_remove {
            self.timers.remove(&id);
        }

        for &id in &ready {
            if let Some(entry) = self.timers.get_mut(&id) {
                if entry.is_interval {
                    entry.deadline = now + Duration::from_millis(entry.delay_ms as u64);
                }
            }
        }

        ready
    }

    pub fn time_until_next(&self) -> Option<Duration> {
        let now = Instant::now();
        self.timers
            .values()
            .map(|e| e.deadline.saturating_duration_since(now))
            .min()
    }
}

pub fn poll_timers(timer_state: &RwLock<TimerState>) -> (Vec<u32>, Option<Duration>) {
    let mut ts = timer_state.write();
    let ready = ts.poll_ready();
    let next = ts.time_until_next();
    (ready, next)
}

// ============================================================================
// Skill Context
// ============================================================================

#[derive(Clone)]
pub struct SkillContext {
    pub skill_id: String,
    pub data_dir: PathBuf,
    pub app_handle: Option<AppHandle>,
}

// ============================================================================
// Skill State (shared published state)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillState {
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

#[derive(Debug)]
pub struct WebSocketConnection {
    pub url: String,
}

#[derive(Debug, Default)]
pub struct WebSocketState {
    pub connections: HashMap<u32, WebSocketConnection>,
    pub next_id: u32,
}

// ============================================================================
// Constants & Helpers
// ============================================================================

pub const ALLOWED_ENV_VARS: &[&str] = &["VITE_BACKEND_URL", "BACKEND_URL", "JWT_TOKEN", "NODE_ENV"];

/// Sanitize error message for use with QuickJS/rquickjs.
/// Some messages (e.g. from SQLite "Invalid symbol 45, offset 19") can trigger
/// "Invalid symbol" when rquickjs creates the JS exception — avoid comma and hyphen.
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

pub fn js_err(msg: impl AsRef<str>) -> rquickjs::Error {
    let sanitized = sanitize_error_message(msg.as_ref());
    rquickjs::Error::new_from_js_message("skill", "RuntimeError", sanitized)
}
