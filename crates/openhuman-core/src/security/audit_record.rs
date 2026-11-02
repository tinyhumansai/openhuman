//! Host-observed rows written through TinySecurity `RecordAudit2`.

use serde_json::{json, Value};

pub const MAX_COMMAND_CHARS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostAuditKind {
    Execution,
    PolicyDecision,
}

#[derive(Clone, PartialEq)]
pub struct HostAuditEvent {
    pub kind: HostAuditKind,
    pub attribution: String,
    pub arguments: Value,
}

impl std::fmt::Debug for HostAuditEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostAuditEvent")
            .field("kind", &self.kind)
            .field("attribution", &self.attribution)
            .finish_non_exhaustive()
    }
}

impl HostAuditEvent {
    #[must_use]
    pub fn command_execution(
        channel: &str,
        command: &str,
        approved: bool,
        allowed: bool,
        success: bool,
        duration_ms: u64,
    ) -> Self {
        let shown: String = command.chars().take(MAX_COMMAND_CHARS).collect();
        Self {
            kind: HostAuditKind::Execution,
            attribution: channel.to_owned(),
            arguments: json!({
                "command": shown,
                "command_truncated": shown.len() < command.len(),
                "approved": approved,
                "allowed": allowed,
                "success": success,
                "duration_ms": duration_ms,
            }),
        }
    }
}

/// Record a host event for the tenant carried by this exact dispatch.
pub async fn record(
    ctx: &crate::core::runtime::DispatchCtx,
    config: Option<&crate::config::Config>,
    event: HostAuditEvent,
) -> Result<(), String> {
    let loaded;
    let config = match config {
        Some(config) => config,
        None => {
            loaded = ctx.load_config().await?;
            &loaded
        }
    };
    if !config.audit.enabled {
        return Ok(());
    }
    #[cfg(feature = "security-module")]
    {
        let tenant = ctx
            .tenant()
            .map_err(|_| "no tenant scope for the audit row".to_owned())?;
        crate::modules::security::audit::record_for_tenant(config, &tenant, event).await
    }

    #[cfg(not(feature = "security-module"))]
    {
        let _ = (ctx, event);
        Err("TinySecurity audit module is not built into this host".to_owned())
    }
}

#[cfg(test)]
#[path = "audit_record_tests.rs"]
mod tests;
