//! The subjects the host registers with the module's approval engine.
//!
//! The module keeps one approval posture per subject (user, workspace, agent)
//! and refuses a call under an unregistered one. The host serves an unbounded
//! set of subjects (SaaS profiles, embedded agents, the posture variants of
//! one agent), so each is registered when first seen with
//! `RegisterApprovalScope2` and refreshed with `ReplaceApprovalScope2` when the
//! user's configuration changed: the "always allow" tool list and the
//! auto-approve setting. Per-flow trust is the module's own durable state. The module only converts a required approval
//! into an audited automatic one as far as those say.

use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;

use tinysecurity_bus::{
    names, AccessTier, ApprovalGrants2, ApprovalScope2, ApprovalScopeReceipt2,
    RegisterApprovalScopeRequest2, SubjectScope,
};

use super::super::engine::Posture;
use super::rows::{digest_id, Rows};
use super::state::{handle_key, host};
use crate::config::Config;
use crate::modules::security::SecurityProxy;

/// The immutable partial-policy reference every approval row binds to. It
/// never changes with the posture, so a refreshed allowlist does not strand
/// the requests already pending.
const POLICY_REF: &str = "openhuman/approval/v1";

/// The key a subject is registered under.
pub(super) fn subject_key(subject: &SubjectScope) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}",
        subject.user, subject.workspace, subject.agent
    )
}

/// One registered subject.
pub(super) struct ScopeEntry {
    pub(super) subject: SubjectScope,
    pub(super) scope_id: String,
    pub(super) stream_id: String,
    pub(super) rows: Rows,
    handle: OnceLock<SubjectScope>,
    /// Held across every module call that appends an audit event for this
    /// subject, so the tail the host reserves is the tail the module extends.
    pub(super) stream: tokio::sync::Mutex<()>,
    /// What the module currently holds for the subject.
    registered: tokio::sync::Mutex<Option<String>>,
    swept: Mutex<Option<Instant>>,
    /// The subject identifiers the module's audit batches carried the first time,
    /// which every later batch of the stream must repeat.
    audit_scope: OnceLock<SubjectScope>,
}

impl std::fmt::Debug for ScopeEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopeEntry").finish_non_exhaustive()
    }
}

impl ScopeEntry {
    pub(super) fn new(subject: SubjectScope, stream_id: String, rows: Rows) -> Self {
        let scope_id = digest_id(&["scope", &subject.user, &subject.workspace, &subject.agent]);
        Self {
            subject,
            scope_id,
            stream_id,
            rows,
            handle: OnceLock::new(),
            stream: tokio::sync::Mutex::new(()),
            registered: tokio::sync::Mutex::new(None),
            swept: Mutex::new(None),
            audit_scope: OnceLock::new(),
        }
    }

    /// The module's handle for the subject, known once it is registered.
    pub(super) fn handle(&self) -> Option<&SubjectScope> {
        self.handle.get()
    }

    /// Whether `scope` is the subject identification this stream's audit batches
    /// have always carried; the first batch fixes it.
    pub(super) fn audit_scope_matches(&self, scope: &SubjectScope) -> bool {
        self.audit_scope.get_or_init(|| scope.clone()) == scope
    }

    /// Whether a stale-request sweep is due, and mark it done.
    pub(super) fn sweep_due(&self, every: std::time::Duration) -> bool {
        let mut swept = self.swept.lock().unwrap_or_else(PoisonError::into_inner);
        if swept.is_some_and(|at| at.elapsed() < every) {
            return false;
        }
        *swept = Some(Instant::now());
        true
    }

    /// Record the module's handle for the subject and make callbacks resolve it.
    pub(super) async fn bind(self: &Arc<Self>, handle: SubjectScope) {
        if self.handle.get().is_some() {
            return;
        }
        // Callbacks look the subject up by this handle (and its audit stream), so
        // both are known before the first call under the scope can reach them.
        {
            let mut scopes = host().scopes.lock().await;
            scopes
                .by_handle
                .insert(handle_key(&handle), Arc::clone(self));
            scopes
                .by_stream
                .insert(self.stream_id.clone(), Arc::clone(self));
        }
        let _ = self.handle.set(handle);
    }

    /// Make the audit sink resolve this subject's stream before the module's first
    /// event under it. Unlike [`Self::bind`] this needs no approval handle: the
    /// sink names a subject only by its stream.
    pub(super) async fn bind_stream(self: &Arc<Self>) {
        host()
            .scopes
            .lock()
            .await
            .by_stream
            .insert(self.stream_id.clone(), Arc::clone(self));
    }

    /// Make the module hold `posture` for this subject.
    async fn register(
        self: &Arc<Self>,
        proxy: &SecurityProxy,
        posture: &Posture,
        generation: u64,
        preconfigured: bool,
    ) -> Result<(), String> {
        let grants = grants();
        let fingerprint = serde_json::to_string(&(
            posture.require_approval,
            posture.auto_approve_all,
            &grants.tools,
        ))
        .map_err(|error| error.to_string())?;
        let mut registered = self.registered.lock().await;
        if registered.as_deref() == Some(fingerprint.as_str()) {
            return Ok(());
        }
        let first = registered.is_none();
        let request = RegisterApprovalScopeRequest2 {
            scope: ApprovalScope2 {
                scope_id: self.scope_id.clone(),
                subject: self.subject.clone(),
                policy_ref: POLICY_REF.into(),
                generation,
                ceiling: AccessTier::Full,
                require_approval: posture.require_approval,
                auto_approve_all: posture.auto_approve_all,
            },
            grants,
            audit_stream_id: self.stream_id.clone(),
            // The handle keys every callback the module makes for this subject.
            report_handle: true,
        };
        let member = registration_method(first, preconfigured);
        let receipt: ApprovalScopeReceipt2 = proxy
            .call_confidential(member, (request,))
            .await
            .map_err(|error| {
                tracing::warn!(
                    member,
                    error = %error,
                    "[approval::native] the module refused the approval scope"
                );
                format!("TinySecurity {member} failed: {error}")
            })?;
        if receipt.scope_id != self.scope_id {
            return Err("TinySecurity returned a receipt for another approval scope".into());
        }
        let handle = receipt
            .scope_handle
            .ok_or("TinySecurity did not report the approval scope handle")?;
        self.bind(handle).await;
        tracing::debug!(
            scope = %self.scope_id,
            outcome = ?receipt.outcome,
            live_scopes = receipt.live_scopes,
            "[approval::native] approval scope current"
        );
        *registered = Some(fingerprint);
        Ok(())
    }

    /// Register through the exact callback binding used to initialize the
    /// module, keeping API calls and host callbacks on one authenticated peer.
    pub(super) async fn register_with_callback(
        self: &Arc<Self>,
        proxy: &tinybus::module::HostCallbackProxy,
        posture: &Posture,
        generation: u64,
    ) -> Result<(), String> {
        let grants = grants();
        let fingerprint = serde_json::to_string(&(
            posture.require_approval,
            posture.auto_approve_all,
            &grants.tools,
        ))
        .map_err(|error| error.to_string())?;
        let mut registered = self.registered.lock().await;
        if registered.as_deref() == Some(fingerprint.as_str()) {
            return Ok(());
        }
        let first = registered.is_none();
        let request = RegisterApprovalScopeRequest2 {
            scope: ApprovalScope2 {
                scope_id: self.scope_id.clone(),
                subject: self.subject.clone(),
                policy_ref: POLICY_REF.into(),
                generation,
                ceiling: AccessTier::Full,
                require_approval: posture.require_approval,
                auto_approve_all: posture.auto_approve_all,
            },
            grants,
            audit_stream_id: self.stream_id.clone(),
            report_handle: true,
        };
        let member = registration_method(first, false);
        let receipt: ApprovalScopeReceipt2 = proxy
            .call_confidential(member, (request,))
            .await
            .map_err(|error| format!("TinySecurity {member} failed: {error}"))?;
        if receipt.scope_id != self.scope_id {
            return Err("TinySecurity returned a receipt for another approval scope".into());
        }
        let handle = receipt
            .scope_handle
            .ok_or("TinySecurity did not report the approval scope handle")?;
        self.bind(handle).await;
        *registered = Some(fingerprint);
        Ok(())
    }
}

fn registration_method(first: bool, preconfigured: bool) -> &'static str {
    if first && !preconfigured {
        names::methods::REGISTER_APPROVAL_SCOPE2
    } else {
        names::methods::REPLACE_APPROVAL_SCOPE2
    }
}

/// The user's configured tool grants, narrowed to what the module accepts: a name
/// it refuses (a human-only or native tool) can never be granted, so it is dropped
/// here rather than failing the whole registration. Per-flow trust is not part of
/// the posture: the module keeps it durably (`GrantFlowTool2`).
fn grants() -> ApprovalGrants2 {
    ApprovalGrants2 {
        tools: Vec::new(),
        flows: Vec::new(),
    }
}

/// The registered subject for `subject`, registering it or refreshing its
/// posture first.
pub(super) async fn ensure(
    config: &Config,
    proxy: &SecurityProxy,
    subject: SubjectScope,
    posture: &Posture,
    generation: u64,
) -> Result<Arc<ScopeEntry>, String> {
    ensure_with_registration(config, proxy, subject, posture, generation, false).await
}

/// Register a scope that TinySecurity already created from module init config.
/// Its init-time scope has no audit stream, so attach the persisted stream with
/// `ReplaceApprovalScope2` on first use.
pub(super) async fn ensure_preconfigured(
    config: &Config,
    proxy: &SecurityProxy,
    subject: SubjectScope,
    posture: &Posture,
    generation: u64,
) -> Result<Arc<ScopeEntry>, String> {
    ensure_with_registration(config, proxy, subject, posture, generation, true).await
}

async fn ensure_with_registration(
    config: &Config,
    proxy: &SecurityProxy,
    subject: SubjectScope,
    posture: &Posture,
    generation: u64,
    preconfigured: bool,
) -> Result<Arc<ScopeEntry>, String> {
    let key = subject_key(&subject);
    let entry = {
        let mut scopes = host().scopes.lock().await;
        match scopes.by_subject.get(&key) {
            Some(found) => Arc::clone(found),
            None => {
                let rows = Rows::current(config).map_err(|error| error.to_string())?;
                let stream_id = rows.stream_for(&key).await?;
                let entry = Arc::new(ScopeEntry::new(subject, stream_id, rows));
                scopes.by_subject.insert(key, Arc::clone(&entry));
                entry
            }
        }
    };
    entry
        .register(proxy, posture, generation, preconfigured)
        .await?;
    Ok(entry)
}

/// The subject's persisted audit stream, creating it on first use, without
/// registering an approval posture. Sandbox plans and approvals of one subject
/// share the stream, so their events extend one ordered ledger.
pub(super) async fn audit_entry(
    config: &Config,
    subject: SubjectScope,
) -> Result<Arc<ScopeEntry>, String> {
    let key = subject_key(&subject);
    let entry = {
        let mut scopes = host().scopes.lock().await;
        match scopes.by_subject.get(&key) {
            Some(found) => Arc::clone(found),
            None => {
                let rows = Rows::current(config).map_err(|error| error.to_string())?;
                let stream_id = rows.stream_for(&key).await?;
                let entry = Arc::new(ScopeEntry::new(subject, stream_id, rows));
                scopes.by_subject.insert(key, Arc::clone(&entry));
                entry
            }
        }
    };
    entry.bind_stream().await;
    Ok(entry)
}

/// The registered subject whose audit stream is `stream_id`.
pub(super) async fn by_stream(stream_id: &str) -> Option<Arc<ScopeEntry>> {
    host().scopes.lock().await.by_stream.get(stream_id).cloned()
}

/// Find the scope the module already registered for this exact subject.
pub(super) async fn existing(subject: &SubjectScope) -> Option<Arc<ScopeEntry>> {
    host()
        .scopes
        .lock()
        .await
        .by_subject
        .get(&subject_key(subject))
        .cloned()
}

/// The registered subject a module callback names by its handle.
pub(super) async fn by_handle(handle: &SubjectScope) -> Option<Arc<ScopeEntry>> {
    host()
        .scopes
        .lock()
        .await
        .by_handle
        .get(&handle_key(handle))
        .cloned()
}

#[cfg(test)]
#[path = "scopes_tests.rs"]
mod tests;
