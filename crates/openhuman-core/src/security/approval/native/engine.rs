//! `NativeEngine`: the approval lifecycle, asked of the TinySecurity module.

use std::time::Duration;

use async_trait::async_trait;
#[cfg(test)]
use tinybus::Proxy;
use tinysecurity_bus::{
    is_host_tool_name2, names, AccessTier, ApprovalActionRequest2, ApprovalChoice2,
    ApprovalLifecycle2, ApprovalRecordRequest2, ApprovalRow2, AuditCorrelation2, CallerContext,
    Check, DecideRequest2, EvaluateApprovalRequest2, EvaluateApprovalResponse2, EvaluateRequest,
    ExecutionOutcome as ModuleOutcome, FlowTrustList2, FlowTrustOutcome2, FlowTrustReceipt2,
    GrantFlowToolRequest2, GrantToolRequest2, ListFlowTrustRequest2, ListToolTrustRequest2, Origin,
    PolicyInfo, RecordExecutionRequest2, RevokeFlowToolRequest2, RevokeToolByIdRequest2,
    SubjectScope, ToolCall, ToolTrustList2, ToolTrustReceipt2, Verdict,
};

use super::super::engine::{
    ApprovalEngine, CallOrigin, CheckInput, Checked, Choice, Decided, EngineError, Posture, Sealed,
    Variant,
};
use super::super::types::{ApprovalAuditEntry, ExecutionOutcome};
use super::scopes::{self, ScopeEntry};
use super::state::{host, CallEntry, RequestEntry, Ticket};
use crate::config::Config;
use crate::modules::security::policy_translation;
use crate::modules::security::SecurityProxy;

const LOG: &str = "[approval::native]";

/// The deciding actor the module records for a decision made through the host's
/// own authenticated approval surface.
const ACTOR: &str = "operator";

/// How long one module call may take: past the longest callback deadline the
/// module itself enforces (store, audit, prompt, judge), so a slow callback
/// surfaces as the module's own refusal rather than a host timeout.
const CALL_DEADLINE: Duration = Duration::from_secs(25);

/// How often a subject's stale pending requests are expired.
const SWEEP_EVERY: Duration = Duration::from_secs(30);

/// How long past its window a request still held in memory is left to its own
/// timer before the sweep expires it.
const SWEEP_GRACE_MS: u64 = 30_000;

/// Stale requests expired per sweep.
const SWEEP_BATCH: usize = 25;

/// Ends a call's wait for its prompt however the check returns.
struct CallGuard(String);

impl Drop for CallGuard {
    fn drop(&mut self) {
        host().end_call(&self.0);
    }
}

/// Where the engine gets its bus proxy.
enum Source {
    /// The attested, digest-pinned module the registry loads.
    Production,
    /// A proxy a test supplies for a module it loaded itself.
    #[cfg(test)]
    Fixed(Proxy),
}

/// The approval engine over the module's bus contract.
pub(crate) struct NativeEngine {
    config: Config,
    source: Source,
}

impl std::fmt::Debug for NativeEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeEngine").finish_non_exhaustive()
    }
}

impl Sealed for NativeEngine {}

fn unavailable(message: impl Into<String>) -> EngineError {
    EngineError::Unavailable(message.into())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn opaque() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn module_origin(origin: CallOrigin) -> Origin {
    match origin {
        CallOrigin::Operator => Origin::Operator,
        CallOrigin::Copilot => Origin::Copilot,
        CallOrigin::SubAgent => Origin::SubAgent,
        CallOrigin::Cron => Origin::Cron,
        CallOrigin::TrustedAutomation => Origin::TrustedAutomation,
        CallOrigin::LocalOperator => Origin::LocalOperator,
        CallOrigin::Unlabelled => Origin::Unlabelled,
    }
}

fn module_outcome(outcome: ExecutionOutcome) -> ModuleOutcome {
    match outcome {
        ExecutionOutcome::Success => ModuleOutcome::Succeeded,
        ExecutionOutcome::Failure => ModuleOutcome::Failed,
        ExecutionOutcome::Aborted => ModuleOutcome::Cancelled,
    }
}

impl NativeEngine {
    /// The engine over the registry's module, for `config`.
    pub(crate) fn production(config: Config) -> Self {
        Self {
            config,
            source: Source::Production,
        }
    }

    /// The engine over a module a test loaded itself.
    #[cfg(test)]
    pub(crate) fn on(config: Config, proxy: Proxy) -> Self {
        Self {
            config,
            source: Source::Fixed(proxy),
        }
    }

    pub(super) fn config(&self) -> &Config {
        &self.config
    }

    /// The module's proxy. The first use in a process also carries the legacy
    /// approval history into the module (`history_import`).
    async fn proxy(&self) -> Result<SecurityProxy, EngineError> {
        let proxy = match &self.source {
            Source::Production => {
                crate::modules::security::approval_object(&self.config, CALL_DEADLINE)
                    .await
                    .map_err(EngineError::Unavailable)?
            }
            #[cfg(test)]
            Source::Fixed(proxy) => SecurityProxy::module(
                crate::modules::ops::fixture_client(proxy.clone())
                    .with_timeout(CALL_DEADLINE)
                    .object_client(crate::modules::security::MODULE_ID, names::OBJECT_PATH)
                    .await
                    .map_err(|error| EngineError::Unavailable(error.to_string()))?,
            ),
        };
        if matches!(self.source, Source::Production) {
            #[cfg(feature = "legacy-import")]
            super::history_import::ensure(self, &proxy).await;
        }
        Ok(proxy)
    }

    /// The subject a call runs under: the tenant, the workspace and the agent,
    /// with the variant that selects its posture.
    pub(super) fn subject(
        &self,
        tenant: &crate::core::runtime::Tenant,
        variant: Variant,
    ) -> Result<SubjectScope, EngineError> {
        let mut subject = policy_translation::subject_scope(
            tenant,
            &self.config.workspace_dir,
            crate::core::runtime::is_saas(),
        )
        .map_err(EngineError::Unavailable)?;
        subject.agent.push_str(variant.suffix());
        Ok(subject)
    }

    /// The registered scope a flow's durable trust lives under: the tenant's
    /// strict-flow subject, registered with the restrictive posture if it is new
    /// (the next gated call refreshes it to the user's configuration).
    async fn flow_scope(
        &self,
        proxy: &SecurityProxy,
        tenant: &crate::core::runtime::Tenant,
    ) -> Result<std::sync::Arc<ScopeEntry>, EngineError> {
        let subject = self.subject(tenant, Variant::FlowStrict)?;
        let generation = Self::generation(proxy).await?;
        scopes::ensure(
            &self.config,
            proxy,
            subject,
            &super::super::engine::Posture {
                require_approval: true,
                ..Default::default()
            },
            generation,
        )
        .await
        .map_err(EngineError::Unavailable)
    }

    /// The module's current generation, which every correlation carries.
    pub(super) async fn generation(proxy: &SecurityProxy) -> Result<u64, EngineError> {
        let info: PolicyInfo = proxy
            .call(names::methods::POLICY_INFO, ())
            .await
            .map_err(|_| unavailable("TinySecurity PolicyInfo failed"))?;
        Ok(info.generation)
    }

    /// The correlation of the next audit event of a subject. The caller holds the
    /// subject's stream lock, so the tail read here is the tail the module extends.
    pub(super) async fn correlation(
        scope: &ScopeEntry,
        generation: u64,
    ) -> Result<AuditCorrelation2, EngineError> {
        let expected_tail = scope
            .rows
            .tail(&scope.stream_id)
            .await
            .map_err(EngineError::Unavailable)?;
        Ok(AuditCorrelation2 {
            event_id: opaque(),
            expected_tail,
            generation,
        })
    }

    fn action(
        entry: &RequestEntry,
        request_id: &str,
        audit: AuditCorrelation2,
    ) -> ApprovalActionRequest2 {
        ApprovalActionRequest2 {
            scope_id: entry.scope.scope_id.clone(),
            request_id: request_id.to_owned(),
            request: entry.request.clone(),
            audit,
        }
    }

    /// What a row became after a call failed, so the caller can tell a lost race
    /// from a failure.
    async fn settled(entry: &RequestEntry, request_id: &str) -> bool {
        match entry.scope.rows.row(request_id).await {
            Ok(Some(stored)) => stored.row.state != ApprovalLifecycle2::Pending,
            _ => false,
        }
    }

    /// Expire this subject's requests whose window elapsed and that nobody will
    /// answer. A failure is logged: the sweep only keeps the module's view small.
    async fn sweep(
        &self,
        proxy: &SecurityProxy,
        scope: &std::sync::Arc<ScopeEntry>,
        generation: u64,
    ) {
        if !scope.sweep_due(SWEEP_EVERY) {
            return;
        }
        let stale = match scope.rows.stale(now_ms(), SWEEP_BATCH).await {
            Ok(stale) => stale,
            Err(error) => {
                tracing::debug!(error = %error, "{LOG} stale sweep could not list rows");
                return;
            }
        };
        for stored in stale {
            let request_id = stored.row.request_id;
            // A request still parked in this process expires through its own
            // timer, which also settles the call waiting on it. One that outlived
            // its window by the grace had its park abandoned (a caller bound),
            // so nothing else will ever close it.
            if host().request(&request_id).is_some()
                && now_ms() < stored.row.expires_at_ms.saturating_add(SWEEP_GRACE_MS)
            {
                continue;
            }
            let _guard = scope.stream.lock().await;
            let Ok(audit) = Self::correlation(scope, generation).await else {
                return;
            };
            let outcome: Result<ApprovalRow2, _> = proxy
                .call_confidential(
                    names::methods::EXPIRE_APPROVAL2,
                    (ApprovalRecordRequest2 {
                        scope_id: scope.scope_id.clone(),
                        request_id: request_id.clone(),
                        audit,
                    },),
                )
                .await;
            match outcome {
                Ok(_) => tracing::debug!(request_id = %request_id, "{LOG} swept a stale request"),
                Err(error) => tracing::debug!(
                    request_id = %request_id,
                    error = %error,
                    "{LOG} stale request left for the next sweep"
                ),
            }
        }
    }
}

#[path = "engine_impl.rs"]
mod engine_impl;
