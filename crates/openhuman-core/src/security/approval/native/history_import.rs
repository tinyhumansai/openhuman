//! One-time import of the legacy `approval.db` into the module.
//!
//! Installs that predate the module kept their approval history and their
//! per-flow trust in `approval/approval.db`. The first time the engine talks to
//! the loaded module in a process, this importer feeds both to it:
//!
//! - each decided request becomes inert history through `ImportApprovalHistory2`
//!   (the module keeps a sanitized copy in a denied row and never revives it as a
//!   grant), at most [`HISTORY_LIMIT`] of the most recent; and
//! - each "approve always for this flow" grant becomes durable flow trust through
//!   `GrantFlowTool2`.
//!
//! The legacy file is read from a scratch copy and never modified. The importer
//! runs once per process and is safe to repeat after a restart: the module returns
//! the existing row for a request it already holds and `Unchanged` for a live grant.

use std::{collections::HashSet, path::PathBuf};

use crate::modules::security::SecurityProxy;
use sha2::{Digest, Sha256};
use tinysecurity_bus::{
    names, AccessTier, ApprovalHistoryImport2, ApprovalRecord, ApprovalRow2, ApprovalState,
    CallerContext, ImportTrustItem2, ImportTrustReceipt2, ImportTrustSourceRequest2, Origin,
};

use super::super::engine::{ApprovalEngine, EngineError, Posture, Variant};
use super::super::store_import::{read_legacy, LegacyApproval, LegacyHistory};
use super::engine::NativeEngine;
use super::scopes::{self, ScopeEntry};
use crate::config::Config;
use crate::core::runtime::Tenant;

const LOG: &str = "[approval::history-import]";

/// Most recent decided requests carried over. The history view shows at most
/// this many, and each import is one audited module call.
pub(super) const HISTORY_LIMIT: usize = 500;

/// How long a request lived when the legacy row recorded no expiry.
const DEFAULT_WINDOW_MS: u64 = 600_000;

/// Legacy tool grants whose immutable module receipts have been observed in this
/// process. Failed imports are not cached, so the next gated call retries them.
fn imported_tool_grants() -> &'static tokio::sync::Mutex<HashSet<(String, String)>> {
    static IMPORTED: std::sync::OnceLock<tokio::sync::Mutex<HashSet<(String, String)>>> =
        std::sync::OnceLock::new();
    IMPORTED.get_or_init(|| tokio::sync::Mutex::new(HashSet::new()))
}

/// The workspaces this process already tried, so the importer runs at most once
/// per process (a failure is retried on the next start, not on every call).
fn attempted() -> &'static tokio::sync::Mutex<HashSet<PathBuf>> {
    static ATTEMPTED: std::sync::OnceLock<tokio::sync::Mutex<HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    ATTEMPTED.get_or_init(|| tokio::sync::Mutex::new(HashSet::new()))
}

/// What one run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Report {
    pub(super) history: usize,
    pub(super) trust: usize,
    pub(super) skipped: usize,
}

/// Run the importer if this process has not tried it for the workspace yet.
/// Never fails the caller: the module and the gate work without the history.
pub(super) async fn ensure(engine: &NativeEngine, proxy: &SecurityProxy) {
    if crate::core::runtime::is_saas() {
        return;
    }
    let workspace = engine.config().workspace_dir.clone();
    // Claimed before the run and released at once: the run itself calls back into
    // the engine, which reaches this function again and must find the claim taken.
    if !attempted().lock().await.insert(workspace) {
        return;
    }
    match run(engine, proxy).await {
        Ok(Some(report)) => tracing::info!(
            history = report.history,
            trust = report.trust,
            skipped = report.skipped,
            "{LOG} imported the legacy approval history into the module"
        ),
        Ok(None) => tracing::debug!("{LOG} nothing to import"),
        Err(error) => tracing::warn!(
            error = %error,
            "{LOG} the legacy approval history was not imported; it is retried on the next start"
        ),
    }
}

/// The module state an imported legacy row maps to.
pub(super) fn state_of(row: &LegacyApproval) -> ApprovalState {
    match row.decision.as_deref() {
        Some(decision) if decision.starts_with("approve") => {
            if row.executed {
                ApprovalState::Executed
            } else {
                ApprovalState::Allowed
            }
        }
        Some(_) => ApprovalState::Denied,
        // An undecided request of an earlier launch can never be resumed.
        None => ApprovalState::Expired,
    }
}

fn millis(at: chrono::DateTime<chrono::Utc>) -> u64 {
    u64::try_from(at.timestamp_millis()).unwrap_or(0)
}

/// The legacy wire record the module keeps (sanitized) as history.
pub(super) fn legacy_record(row: &LegacyApproval) -> ApprovalRecord {
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(
            format!(
                "{}\u{1f}{}\u{1f}{}",
                row.request_id,
                row.tool_name,
                row.created_at.timestamp_millis()
            )
            .as_bytes()
        )
    );
    ApprovalRecord {
        request_id: row.request_id.clone(),
        fingerprint,
        caller: CallerContext {
            agent: row.agent_id.clone().unwrap_or_else(|| "operator".into()),
            thread: None,
            flow: None,
            call_id: row.request_id.clone(),
            origin: Origin::Operator,
            tier: AccessTier::Read,
        },
        expires_at_ms: row
            .expires_at
            .map_or_else(|| millis(row.created_at) + DEFAULT_WINDOW_MS, millis),
        state: state_of(row),
    }
}

/// The most recent decided rows, oldest first.
pub(super) fn recent_decided(history: &LegacyHistory) -> Vec<&LegacyApproval> {
    let mut decided: Vec<&LegacyApproval> = history
        .approvals
        .iter()
        .filter(|row| row.decision.is_some())
        .collect();
    decided.sort_by_key(|row| row.decided_at.unwrap_or(row.created_at));
    let skip = decided.len().saturating_sub(HISTORY_LIMIT);
    decided.split_off(skip)
}

async fn read(config: Config) -> Result<Option<LegacyHistory>, EngineError> {
    crate::core::runtime::DispatchCtx::ambient()
        .spawn_blocking(move || read_legacy(&config))
        .await
        .map_err(|error| {
            EngineError::Unavailable(format!("the legacy read did not finish: {error}"))
        })?
        .map_err(|error| EngineError::Unavailable(format!("{error:#}")))
}

pub(super) async fn run(
    engine: &NativeEngine,
    proxy: &SecurityProxy,
) -> Result<Option<Report>, EngineError> {
    let Some(history) = read(engine.config().clone()).await? else {
        return Ok(None);
    };
    let tenant = Tenant::default();
    let mut report = Report::default();
    if !history.approvals.is_empty() {
        let scope = engine.history_scope(proxy, &tenant).await?;
        for row in recent_decided(&history) {
            match import_row(proxy, &scope, row).await {
                Ok(()) => report.history += 1,
                Err(error @ EngineError::Unavailable(_)) => return Err(error),
                Err(_) => report.skipped += 1,
            }
        }
    }
    for grant in &history.trust {
        match engine
            .grant_flow_trust(&tenant, &grant.flow_id, &grant.tool_name)
            .await
        {
            Ok(()) => report.trust += 1,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "{LOG} a legacy flow trust grant was not carried over"
                );
                report.skipped += 1;
            }
        }
    }
    Ok(Some(report))
}

/// Import the legacy `autonomy.auto_approve` entries into durable module grants.
/// Scope registration deliberately carries no transient tool grants; an import
/// receipt makes this safe to retry after restart without reviving a revoked row.
pub(super) async fn import_tool_grants(
    proxy: &SecurityProxy,
    scope: &std::sync::Arc<ScopeEntry>,
    tools: &[String],
) {
    for tool in tools {
        if !tinysecurity_bus::is_host_tool_name2(tool) {
            continue;
        }
        let key = (scope.scope_id.clone(), tool.clone());
        if imported_tool_grants().lock().await.contains(&key) {
            continue;
        }
        let _guard = scope.stream.lock().await;
        let result = async {
            let generation = NativeEngine::generation(proxy)
                .await
                .map_err(|error| error.to_string())?;
            let audit = NativeEngine::correlation(scope, generation)
                .await
                .map_err(|error| error.to_string())?;
            let receipt: ImportTrustReceipt2 = proxy
                .call_confidential(
                    names::methods::IMPORT_TRUST_SOURCE2,
                    (ImportTrustSourceRequest2 {
                        scope_id: scope.scope_id.clone(),
                        source_id: "openhuman.autonomy.auto_approve".into(),
                        source_version: "1".into(),
                        item: ImportTrustItem2::Tool { tool: tool.clone() },
                        audit,
                    },),
                )
                .await
                .map_err(|error| format!("TinySecurity ImportTrustSource2 failed: {error}"))?;
            if receipt.scope_id != scope.scope_id {
                return Err("TinySecurity returned a receipt for another approval scope".into());
            }
            Ok::<(), String>(())
        }
        .await;
        match result {
            Ok(()) => {
                imported_tool_grants().lock().await.insert(key);
            }
            Err(error) => tracing::warn!(
                tool,
                error = %error,
                "{LOG} legacy tool trust was not imported; the tool will require approval"
            ),
        }
    }
}

#[cfg(test)]
#[path = "history_import_tests.rs"]
mod tests;

async fn import_row(
    proxy: &SecurityProxy,
    scope: &std::sync::Arc<ScopeEntry>,
    row: &LegacyApproval,
) -> Result<(), EngineError> {
    let _guard = scope.stream.lock().await;
    let generation = NativeEngine::generation(proxy).await?;
    let audit = NativeEngine::correlation(scope, generation).await?;
    let _: ApprovalRow2 = proxy
        .call_confidential(
            names::methods::IMPORT_APPROVAL_HISTORY2,
            (ApprovalHistoryImport2 {
                scope_id: scope.scope_id.clone(),
                legacy: legacy_record(row),
                audit,
            },),
        )
        .await
        .map_err(|error| {
            tracing::debug!(error = %error, "{LOG} the module refused a legacy row");
            EngineError::Unavailable(format!(
                "TinySecurity ImportApprovalHistory2 failed: {error}"
            ))
        })?;
    Ok(())
}

impl NativeEngine {
    /// The operator's standard subject, registered with the restrictive posture,
    /// which the imported history lives under.
    pub(super) async fn history_scope(
        &self,
        proxy: &SecurityProxy,
        tenant: &Tenant,
    ) -> Result<std::sync::Arc<ScopeEntry>, EngineError> {
        let subject = self.subject(tenant, Variant::Standard)?;
        let generation = Self::generation(proxy).await?;
        scopes::ensure(
            self.config(),
            proxy,
            subject,
            &Posture {
                require_approval: true,
                ..Posture::default()
            },
            generation,
        )
        .await
        .map_err(EngineError::Unavailable)
    }
}
