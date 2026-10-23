//! The host's security audit rows, written to the TinySecurity module's one
//! audit stream.
//!
//! The module owns the security audit trail: every row of a subject (approvals,
//! judge verdicts, sandbox plans, and the rows below) is an event in that
//! subject's ordered, contiguous stream that the module sanitizes and the host
//! persists behind `AuditSink2` (`security::approval::native::rows`). The host
//! keeps no second log. What the host itself observes, and the module cannot, is
//! recorded here through the module's `RecordAudit2`:
//!
//! - a shell command's execution result (`tool:shell`); and
//! - an external-effect tool's terminal result (`tool:<registered-name>`).
//!
//! A row the module cannot record is an `Err`. The caller logs it at warn level;
//! there is no fallback sink and the row is never dropped silently.

use serde_json::{json, Value};

/// Most characters of a command an audit row carries. The module sanitizes the
/// text; this bounds how much of what the user ran a row repeats.
pub const MAX_COMMAND_CHARS: usize = 512;

/// What a host row records. Only these two kinds are accepted from the host; the
/// module writes every other kind itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostAuditKind {
    /// A terminal execution result.
    Execution,
    /// A call let through without a prompt.
    PolicyDecision,
}

/// One host-observed row, before the module sanitizes it.
#[derive(Clone, PartialEq)]
pub struct HostAuditEvent {
    pub kind: HostAuditKind,
    /// Who acted, content-free (`tool:shell`).
    pub attribution: String,
    /// Structured arguments; the module redacts them before they are stored.
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
    /// The content-free terminal outcome of an external-effect tool call.
    /// Tool arguments and returned content are deliberately excluded.
    #[must_use]
    pub fn external_tool_execution(tool_name: &str, outcome: &str) -> Self {
        Self {
            kind: HostAuditKind::Execution,
            attribution: format!("tool:{tool_name}"),
            arguments: json!({ "outcome": outcome }),
        }
    }

    /// The row of one shell command: whether it was allowed to run, whether it
    /// needed a human approval, and how it ended.
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

/// Append `event` to the audit stream of the calling tenant's subject.
///
/// `config` is the configuration the caller already holds; without one the
/// context's is loaded. Every failure (no module, no tenant, a refused or
/// uncommitted append) is returned for the caller to log.
#[cfg(feature = "security-module")]
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
    let tenant = ctx
        .tenant()
        .map_err(|_| "no tenant scope for the audit row".to_owned())?;
    crate::modules::security::audit::record_for_tenant(config, &tenant, event).await
}

/// Record through an explicit module connection, retaining the dispatch's tenant.
/// The native fixture uses this to test tenant selection without mutating global state.
#[cfg(feature = "security-module")]
pub(crate) async fn record_with_proxy(
    ctx: &crate::core::runtime::DispatchCtx,
    config: Option<&crate::config::Config>,
    event: HostAuditEvent,
    proxy: &tinybus::Proxy,
) -> Result<(), String> {
    let loaded;
    let config = match config {
        Some(config) => config,
        None => {
            loaded = ctx.load_config().await?;
            &loaded
        }
    };
    let tenant = ctx
        .tenant()
        .map_err(|_| "no tenant scope for the audit row".to_owned())?;
    crate::modules::security::audit::record_for_tenant_with(proxy, config, &tenant, event).await
}

/// Without the module there is no audit stream; the row is reported, not kept.
#[cfg(not(feature = "security-module"))]
pub async fn record(
    _ctx: &crate::core::runtime::DispatchCtx,
    _config: Option<&crate::config::Config>,
    _event: HostAuditEvent,
) -> Result<(), String> {
    Err("the TinySecurity module is not built into this binary".into())
}

#[cfg(test)]
#[path = "audit_record_tests.rs"]
mod tests;
