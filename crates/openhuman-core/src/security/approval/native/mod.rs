//! The native approval engine: the TinySecurity module behind its bus contract.
//!
//! [`NativeEngine`] implements the gate's [`ApprovalEngine`](super::engine) by
//! registering each subject with the module, allocating the audit correlation of
//! each phase, and calling `EvaluateApproval2`, `Decide2`, `ConsumeApproval2`,
//! `RecordExecution2`, `ExpireApproval2` and `CancelApproval2`. The module
//! calls back into the host through [`callbacks`], which serve its store, its
//! audit sink, the prompt and the human verification from this directory's
//! state.
//!
//! Layout:
//!
//! | File | Role |
//! | --- | --- |
//! | `engine.rs` | `NativeEngine`, the module calls |
//! | `scopes.rs` | subject registration, posture and grants |
//! | `audit_stream.rs` | a subject's audit stream, leased by the sandbox planner |
//! | `rows.rs` | the document-port store behind `Load`, `Cas` and `Append` |
//! | `history_import.rs` | one-time import of the legacy `approval.db` into the module |
//! | `callbacks.rs` | what `modules/security_host.rs` serves |
//! | `state.rs` | calls, requests and human tickets in flight |
//! | `view.rs` | history read from the stored rows |

use self::state::host;
use sha2::{Digest, Sha256};

pub(crate) mod audit_stream;
pub(crate) mod callbacks;
mod engine;
#[cfg(feature = "legacy-import")]
mod history_import;
mod rows;
mod scopes;
mod state;
mod view;

pub(crate) use engine::NativeEngine;

/// A host-owned secret approval scope with its ordered audit stream.
///
/// The scope entry remains private to the approval subsystem; callers receive
/// only the identifiers needed to initialize TinySecurity and an audit lease.
#[derive(Clone)]
pub(crate) struct SecretScope {
    entry: std::sync::Arc<scopes::ScopeEntry>,
}

impl SecretScope {
    /// The subject's immutable approval scope identifier.
    pub(crate) fn scope_id(&self) -> &str {
        &self.entry.scope_id
    }

    /// The subject identity used by the approval and audit ledgers.
    pub(crate) fn subject(&self) -> &tinysecurity_bus::SubjectScope {
        &self.entry.subject
    }

    pub(crate) fn matches_handle(&self, handle: &tinysecurity_bus::SubjectScope) -> bool {
        self.entry.handle() == Some(handle)
    }

    /// The persisted audit stream configured in TinySecurity at initialization.
    pub(crate) fn stream_id(&self) -> &str {
        &self.entry.stream_id
    }

    /// Lease the subject stream while preparing and submitting one native call.
    pub(crate) async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.entry.stream.lock().await
    }

    pub(crate) fn begin_prompt(
        &self,
        prompt: std::sync::Arc<dyn super::engine::PromptSurface>,
    ) -> bool {
        host().begin_secret_prompt(std::sync::Arc::clone(&self.entry), prompt)
    }

    /// Allocate a correlation for the next audit event while holding `lock`.
    pub(crate) async fn correlation(
        &self,
        generation: u64,
    ) -> Result<tinysecurity_bus::AuditCorrelation2, String> {
        let expected_tail = self.entry.rows.tail(&self.entry.stream_id).await?;
        Ok(tinysecurity_bus::AuditCorrelation2 {
            event_id: uuid::Uuid::new_v4().simple().to_string(),
            expected_tail,
            generation,
        })
    }

    /// Reserve ordered audit positions for one native operation while holding
    /// the scope stream lock. The caller must submit events in returned order.
    pub(crate) async fn correlations(
        &self,
        generation: u64,
        count: usize,
    ) -> Result<Vec<tinysecurity_bus::AuditCorrelation2>, String> {
        let tail = self.entry.rows.tail(&self.entry.stream_id).await?;
        (0..count)
            .map(|offset| {
                let offset = u64::try_from(offset).map_err(|_| "audit sequence overflow")?;
                Ok(tinysecurity_bus::AuditCorrelation2 {
                    event_id: uuid::Uuid::new_v4().simple().to_string(),
                    expected_tail: tail.checked_add(offset).ok_or("audit sequence overflow")?,
                    generation,
                })
            })
            .collect()
    }

    /// Register or refresh this subject with the admitted TinySecurity module.
    pub(crate) async fn ensure_registered(
        &self,
        config: &crate::config::Config,
        proxy: &crate::modules::security::SecurityProxy,
        generation: u64,
    ) -> Result<(), String> {
        scopes::ensure(
            config,
            proxy,
            self.entry.subject.clone(),
            &super::engine::Posture {
                require_approval: true,
                ..Default::default()
            },
            generation,
        )
        .await
        .map(|_| ())
    }

    /// Forget the module's registration of this scope once it was retired.
    pub(crate) async fn forget_registration(&self) {
        self.entry.forget_registration().await;
        permission_ledgers()
            .tenants
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(self.scope_id());
    }

    /// Register this runtime scope (with its audit stream) through the
    /// callback-bound TinySecurity client used for this module instance.
    pub(crate) async fn ensure_registered_with_callback(
        &self,
        config: &crate::config::Config,
        proxy: &tinybus::module::HostCallbackProxy,
        generation: u64,
    ) -> Result<(), String> {
        let proxy = crate::modules::security::SecurityProxy::callback(proxy.clone());
        let entry = scopes::ensure(
            config,
            &proxy,
            self.entry.subject.clone(),
            &super::engine::Posture {
                require_approval: true,
                ..Default::default()
            },
            generation,
        )
        .await?;
        if !std::sync::Arc::ptr_eq(&entry, &self.entry) {
            return Err("TinySecurity approval scope changed during initialization".into());
        }
        Ok(())
    }
}

/// Prepare the real persisted scope and audit stream before module admission.
/// The returned identifiers may be embedded in init configuration; the scope
/// is registered with TinySecurity after the module is loaded.
pub(crate) async fn prepare_secret_scope(
    config: &crate::config::Config,
    subject: tinysecurity_bus::SubjectScope,
) -> Result<SecretScope, String> {
    let entry = scopes::audit_entry(config, subject).await?;
    Ok(SecretScope { entry })
}

/// One tenant's ledger of audited permissions (path, command and egress checks).
///
/// TinySecurity appends every audited permission on its one init audit stream,
/// numbering each event from the correlation the host hands out. The host
/// keeps a ledger of that stream in each tenant's own store, under the tenant's
/// own tail, and routes every batch back to the tenant whose correlation it
/// carries ([`route_permission_batch`]). No tenant's checks are written into
/// another's store, and a batch the host never correlated is refused.
#[derive(Clone)]
pub(crate) struct PermissionAudit {
    entry: std::sync::Arc<scopes::ScopeEntry>,
}

impl PermissionAudit {
    /// Lease the tenant's ledger while preparing and submitting one call.
    pub(crate) async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.entry.stream.lock().await
    }

    /// Allocate the next event's correlation, routed to this tenant's ledger
    /// until [`Self::settle`].
    pub(crate) async fn correlation(
        &self,
        generation: u64,
    ) -> Result<tinysecurity_bus::AuditCorrelation2, String> {
        let expected_tail = self.entry.rows.tail(&self.entry.stream_id).await?;
        let event_id = uuid::Uuid::new_v4().simple().to_string();
        permission_ledgers()
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(event_id.clone(), std::sync::Arc::clone(&self.entry));
        Ok(tinysecurity_bus::AuditCorrelation2 {
            event_id,
            expected_tail,
            generation,
        })
    }

    /// Stop routing `event_id` once its call returned.
    pub(crate) fn settle(&self, event_id: &str) {
        permission_ledgers()
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(event_id);
    }
}

/// The tenant ledgers of the init audit stream, and the correlations in flight.
#[derive(Default)]
struct PermissionLedgers {
    /// By the tenant's approval scope id; dropped when the tenant retires.
    tenants:
        std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<scopes::ScopeEntry>>>,
    /// By the event id of a correlation handed out and not yet settled.
    events: std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<scopes::ScopeEntry>>>,
}

fn permission_ledgers() -> &'static PermissionLedgers {
    static LEDGERS: std::sync::OnceLock<PermissionLedgers> = std::sync::OnceLock::new();
    LEDGERS.get_or_init(PermissionLedgers::default)
}

/// `tenant`'s permission ledger on the init audit stream (`subject`,
/// `stream_id`), kept in the tenant's own store.
pub(crate) fn permission_audit(
    tenant: &SecretScope,
    subject: tinysecurity_bus::SubjectScope,
    stream_id: &str,
) -> PermissionAudit {
    let mut tenants = permission_ledgers()
        .tenants
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let entry = tenants
        .entry(tenant.scope_id().to_owned())
        .or_insert_with(|| {
            tracing::debug!("[security][audit] tenant permission ledger opened");
            std::sync::Arc::new(scopes::ScopeEntry::new(
                subject,
                stream_id.to_owned(),
                tenant.entry.rows.clone(),
            ))
        });
    PermissionAudit {
        entry: std::sync::Arc::clone(entry),
    }
}

/// The tenant ledger a batch on the init audit stream belongs to: the one every
/// event's correlation was handed out for. Anything else is refused.
fn route_permission_batch(
    request: &tinysecurity_bus::AuditSinkRequest2,
) -> Option<std::sync::Arc<scopes::ScopeEntry>> {
    let events = permission_ledgers()
        .events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut owner: Option<&std::sync::Arc<scopes::ScopeEntry>> = None;
    for event in &request.events {
        let entry = events.get(&event.event_id)?;
        if owner.is_some_and(|owner| !std::sync::Arc::ptr_eq(owner, entry)) {
            return None;
        }
        owner = Some(entry);
    }
    owner
        .filter(|entry| entry.stream_id == request.stream_id)
        .cloned()
}

/// Take the exact native master operation after the shared approval gate returns.
pub(crate) fn take_secret_grant(
    registration_id: &str,
) -> Option<(tinysecurity_bus::SecretRequest2, String)> {
    host().take_secret_grant(registration_id)
}

pub(crate) fn forget_secret_grant(registration_id: &str) {
    host().forget_secret_grant(registration_id);
}

#[cfg(test)]
#[path = "secret_scope_tests.rs"]
mod secret_scope_tests;

/// The SHA-256 correlation of a ticket the module stores instead of the ticket.
pub(super) fn ticket_sha256(ticket: &str) -> String {
    format!("{:x}", Sha256::digest(ticket.as_bytes()))
}
