//! @alphahuman/log bridge — structured logging from JS skills.
//!
//! Exposes log levels: debug, info, warn, error.
//! Logs are forwarded to Rust's `log` crate AND emitted as Tauri events
//! so the frontend can display them.

use serde::{Deserialize, Serialize};

/// A log entry produced by a skill.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillLogEntry {
    pub skill_id: String,
    pub level: LogLevel,
    pub message: String,
    pub timestamp: String,
}

/// Log severity levels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// Log a message from a skill. Forwards to Rust `log` crate with skill context.
pub fn skill_log(skill_id: &str, level: LogLevel, message: &str) {
    let prefixed = format!("[skill:{}] {}", skill_id, message);
    match level {
        LogLevel::Debug => log::debug!("{}", prefixed),
        LogLevel::Info => log::info!("{}", prefixed),
        LogLevel::Warn => log::warn!("{}", prefixed),
        LogLevel::Error => log::error!("{}", prefixed),
    }
}

/// Create a `SkillLogEntry` with the current timestamp.
#[allow(dead_code)]
pub fn make_log_entry(skill_id: &str, level: LogLevel, message: &str) -> SkillLogEntry {
    SkillLogEntry {
        skill_id: skill_id.to_string(),
        level,
        message: message.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    }
}
