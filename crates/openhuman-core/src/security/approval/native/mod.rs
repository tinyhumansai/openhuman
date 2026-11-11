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

    /// Register through the callback-bound TinySecurity client used for this
    /// module instance.
    pub(crate) async fn ensure_registered_with_callback(
        &self,
        config: &crate::config::Config,
        proxy: &tinybus::module::HostCallbackProxy,
        generation: u64,
    ) -> Result<(), String> {
        let proxy = crate::modules::security::SecurityProxy::callback(proxy.clone());
        let entry = scopes::ensure_preconfigured(
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
