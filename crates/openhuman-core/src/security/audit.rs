//! Host-side workspace registry for the module-owned audit stream.

use crate::config::AuditConfig;
use anyhow::Result;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

/// Host handle retaining the audit's enabled setting for tool assembly.
/// Event storage and ordering belong to TinySecurity and its `AuditSink2`.
pub struct AuditSink {
    enabled: AtomicBool,
}

static WORKSPACE_AUDIT_SINKS: OnceLock<Mutex<HashMap<PathBuf, Arc<AuditSink>>>> = OnceLock::new();

pub fn get_or_create_workspace_audit_sink(
    config: AuditConfig,
    openhuman_dir: PathBuf,
) -> Result<Arc<AuditSink>> {
    let workspace = std::fs::canonicalize(&openhuman_dir).unwrap_or(openhuman_dir);
    let registry = WORKSPACE_AUDIT_SINKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = registry.lock();
    if let Some(existing) = map.get(&workspace) {
        existing.enabled.store(config.enabled, Ordering::Release);
        return Ok(Arc::clone(existing));
    }
    let logger = Arc::new(AuditSink::new(config, workspace.clone())?);
    map.insert(workspace, Arc::clone(&logger));
    Ok(logger)
}

impl AuditSink {
    pub fn disabled() -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(false),
        })
    }

    pub fn new(config: AuditConfig, _workspace: PathBuf) -> Result<Self> {
        Ok(Self {
            enabled: AtomicBool::new(config.enabled),
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
