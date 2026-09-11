//! Audit logging for security events

use crate::openhuman::config::AuditConfig;
use anyhow::Result;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

/// Audit event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    CommandExecution,
    FileAccess,
    ConfigChange,
    AuthSuccess,
    AuthFailure,
    PolicyViolation,
    SecurityEvent,
}

/// Actor information (who performed the action)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {
    pub channel: String,
    pub user_id: Option<String>,
    pub username: Option<String>,
}

/// Action information (what was done)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub command: Option<String>,
    pub risk_level: Option<String>,
    pub approved: bool,
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
}

/// Execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
}

/// Security context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityContext {
    pub policy_violation: bool,
    pub rate_limit_remaining: Option<u32>,
    pub sandbox_backend: Option<String>,
}

/// Complete audit event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub timestamp: DateTime<Utc>,
    pub event_id: String,
    pub event_type: AuditEventType,
    pub actor: Option<Actor>,
    pub action: Option<Action>,
    pub result: Option<ExecutionResult>,
    pub security: SecurityContext,
}

impl AuditEvent {
    /// Create a new audit event
    pub fn new(event_type: AuditEventType) -> Self {
        Self {
            timestamp: Utc::now(),
            event_id: Uuid::new_v4().to_string(),
            event_type,
            actor: None,
            action: None,
            result: None,
            security: SecurityContext {
                policy_violation: false,
                rate_limit_remaining: None,
                sandbox_backend: None,
            },
        }
    }

    /// Set the actor
    pub fn with_actor(
        mut self,
        channel: String,
        user_id: Option<String>,
        username: Option<String>,
    ) -> Self {
        self.actor = Some(Actor {
            channel,
            user_id,
            username,
        });
        self
    }

    /// Set the action
    pub fn with_action(
        mut self,
        command: String,
        risk_level: String,
        approved: bool,
        allowed: bool,
    ) -> Self {
        self.action = Some(Action {
            command: Some(command),
            risk_level: Some(risk_level),
            approved,
            allowed,
            provider_id: None,
            capability_id: None,
            policy_decision: None,
            approval_id: None,
        });
        self
    }

    /// Set action metadata for a generated tool execution.
    pub fn with_generated_tool_action(mut self, entry: GeneratedToolExecutionLog<'_>) -> Self {
        self.action = Some(Action {
            command: Some(entry.tool_name.to_string()),
            risk_level: Some(entry.risk_level.to_string()),
            approved: entry.approved,
            allowed: entry.allowed,
            provider_id: Some(entry.provider_id.to_string()),
            capability_id: Some(entry.capability_id.to_string()),
            policy_decision: Some(entry.policy_decision.to_string()),
            approval_id: entry.approval_id.map(str::to_string),
        });
        self
    }

    /// Set the result
    pub fn with_result(
        mut self,
        success: bool,
        exit_code: Option<i32>,
        duration_ms: u64,
        error: Option<String>,
    ) -> Self {
        self.result = Some(ExecutionResult {
            success,
            exit_code,
            duration_ms: Some(duration_ms),
            error,
        });
        self
    }

    /// Set security context
    pub fn with_security(mut self, sandbox_backend: Option<String>) -> Self {
        self.security.sandbox_backend = sandbox_backend;
        self
    }
}

/// Audit logger
pub struct AuditLogger {
    log_path: PathBuf,
    config: AuditConfig,
    buffer: Mutex<Vec<AuditEvent>>,
    /// Serializes the rotate + append + fsync critical section so concurrent
    /// callers sharing one logger cannot interleave partial JSON lines.
    write_lock: Mutex<()>,
}

/// Process-global cache of one [`AuditLogger`] per workspace directory.
///
/// Multiple agent sessions in the same process share a workspace and therefore
/// the same `<workspace>/audit.log`. Handing each session its own logger would
/// let concurrent appends race on rotation and interleave partial lines; this
/// registry guarantees a single `Arc<AuditLogger>` per workspace so all writes
/// serialize through one instance's `write_lock`.
static WORKSPACE_AUDIT_LOGGERS: OnceLock<Mutex<HashMap<PathBuf, Arc<AuditLogger>>>> =
    OnceLock::new();

/// Return the shared [`AuditLogger`] for `openhuman_dir`, creating it on first
/// use. All callers targeting the same workspace receive the same `Arc`, so log
/// rotation and appends coordinate through one logger. The `config` of the first
/// caller for a given workspace wins; later callers reuse the cached logger.
pub fn get_or_create_workspace_audit_logger(
    config: AuditConfig,
    openhuman_dir: PathBuf,
) -> Result<Arc<AuditLogger>> {
    // Normalize the key: `PathBuf` equality is lexical, so `/ws` vs `/ws/` vs a
    // symlinked spelling would otherwise cache distinct loggers for one physical
    // workspace and reopen the rotate/append race this registry prevents.
    //
    // A canonicalize failure falls back to the raw path rather than propagating:
    // audit-logger creation must never block agent startup. `NotFound` (the
    // workspace dir not created yet) is expected and logged at debug; other
    // errors (permission, I/O) are unexpected and logged at warn so real
    // filesystem problems stay observable.
    let openhuman_dir = match std::fs::canonicalize(&openhuman_dir) {
        Ok(path) => path,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            log::debug!(
                "[openhuman:audit] workspace path not yet created; keying registry on raw path: {}",
                openhuman_dir.display()
            );
            openhuman_dir
        }
        Err(err) => {
            log::warn!(
                "[openhuman:audit] failed to canonicalize workspace path {} ({err}); keying registry on raw path",
                openhuman_dir.display()
            );
            openhuman_dir
        }
    };
    let registry = WORKSPACE_AUDIT_LOGGERS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = registry.lock();
    if let Some(existing) = map.get(&openhuman_dir) {
        return Ok(Arc::clone(existing));
    }
    let logger = Arc::new(AuditLogger::new(config, openhuman_dir.clone())?);
    map.insert(openhuman_dir, Arc::clone(&logger));
    Ok(logger)
}

/// Structured command execution details for audit logging.
#[derive(Debug, Clone)]
pub struct CommandExecutionLog<'a> {
    pub channel: &'a str,
    pub command: &'a str,
    pub risk_level: &'a str,
    pub approved: bool,
    pub allowed: bool,
    pub success: bool,
    pub duration_ms: u64,
}

/// Structured generated tool execution details for audit correlation.
#[derive(Debug, Clone)]
pub struct GeneratedToolExecutionLog<'a> {
    pub channel: &'a str,
    pub tool_name: &'a str,
    pub provider_id: &'a str,
    pub capability_id: &'a str,
    pub risk_level: &'a str,
    pub policy_decision: &'a str,
    pub approval_id: Option<&'a str>,
    pub approved: bool,
    pub allowed: bool,
    pub success: bool,
    pub duration_ms: u64,
}

impl AuditLogger {
    /// Build a disabled `Arc<AuditLogger>` for tests and contexts that need a
    /// handle but should not write to disk. The `enabled = false` flag
    /// short-circuits `log()` before any filesystem I/O, so the sentinel
    /// `log_path` is never touched.
    pub fn disabled() -> Arc<Self> {
        Arc::new(Self {
            log_path: PathBuf::new(),
            config: AuditConfig {
                enabled: false,
                log_path: String::new(),
                max_size_mb: 0,
            },
            buffer: Mutex::new(Vec::new()),
            write_lock: Mutex::new(()),
        })
    }

    /// Create a new audit logger
    pub fn new(config: AuditConfig, openhuman_dir: PathBuf) -> Result<Self> {
        let log_path = openhuman_dir.join(&config.log_path);
        log::info!(
            "[openhuman:audit] Logger initialized: enabled={}, path={}",
            config.enabled,
            log_path.display()
        );
        Ok(Self {
            log_path,
            config,
            buffer: Mutex::new(Vec::new()),
            write_lock: Mutex::new(()),
        })
    }

    /// Log an event
    pub fn log(&self, event: &AuditEvent) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        log::debug!(
            "[openhuman:audit] Logging event: type={:?}, id={}",
            event.event_type,
            event.event_id
        );

        // Serialize the event outside the write lock — formatting is pure.
        let line = serde_json::to_string(event)?;

        // Hold `write_lock` across rotate + append + fsync so concurrent
        // callers sharing this logger cannot interleave partial lines or
        // race on rotation renames.
        let _guard = self.write_lock.lock();
        self.rotate_if_needed()?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)?;

        writeln!(file, "{}", line)?;
        file.sync_all()?;

        Ok(())
    }

    /// Log a command execution event.
    pub fn log_command_event(&self, entry: CommandExecutionLog<'_>) -> Result<()> {
        let event = AuditEvent::new(AuditEventType::CommandExecution)
            .with_actor(entry.channel.to_string(), None, None)
            .with_action(
                entry.command.to_string(),
                entry.risk_level.to_string(),
                entry.approved,
                entry.allowed,
            )
            .with_result(entry.success, None, entry.duration_ms, None);

        self.log(&event)
    }

    /// Log a generated tool execution event with provider/capability
    /// provenance suitable for runtime policy audits.
    pub fn log_generated_tool_event(&self, entry: GeneratedToolExecutionLog<'_>) -> Result<()> {
        let event = AuditEvent::new(AuditEventType::CommandExecution)
            .with_actor(entry.channel.to_string(), None, None)
            .with_generated_tool_action(entry.clone())
            .with_result(entry.success, None, entry.duration_ms, None);

        self.log(&event)
    }

    /// Backward-compatible helper to log a command execution event.
    #[allow(clippy::too_many_arguments)]
    pub fn log_command(
        &self,
        channel: &str,
        command: &str,
        risk_level: &str,
        approved: bool,
        allowed: bool,
        success: bool,
        duration_ms: u64,
    ) -> Result<()> {
        self.log_command_event(CommandExecutionLog {
            channel,
            command,
            risk_level,
            approved,
            allowed,
            success,
            duration_ms,
        })
    }

    /// Rotate log if it exceeds max size
    fn rotate_if_needed(&self) -> Result<()> {
        if let Ok(metadata) = std::fs::metadata(&self.log_path) {
            let current_size_mb = metadata.len() / (1024 * 1024);
            if current_size_mb >= u64::from(self.config.max_size_mb) {
                self.rotate()?;
            }
        }
        Ok(())
    }

    /// Rotate the log file
    fn rotate(&self) -> Result<()> {
        log::info!(
            "[openhuman:audit] Rotating audit log: {}",
            self.log_path.display()
        );
        for i in (1..10).rev() {
            let old_name = format!("{}.{}.log", self.log_path.display(), i);
            let new_name = format!("{}.{}.log", self.log_path.display(), i + 1);
            let _ = std::fs::rename(&old_name, &new_name);
        }

        let rotated = format!("{}.1.log", self.log_path.display());
        std::fs::rename(&self.log_path, &rotated)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
