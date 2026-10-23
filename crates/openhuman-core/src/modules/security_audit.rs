//! Host rows on a subject's audit stream: the client of the module's
//! `RecordAudit2`.
//!
//! The module appends approval, judge and sandbox events to a subject's stream
//! itself. What only the host sees (shell and external-effect tool results) is recorded here:
//! the host reserves the next position of the subject's stream under the stream
//! lock, hands the module the exact correlation, and the module sanitizes the
//! row and appends it through the same `AuditSink2` callback. An answer without a
//! committed receipt is an `Err`: the row was not recorded and nothing else
//! records it.

use tinysecurity_bus::{
    names, AuditCorrelation2, AuditKind2, AuditRecordRequest2, AuditResponse2, SubjectScope,
};

use crate::config::Config;
use crate::security::approval::native::audit_stream::AuditStream;
use crate::security::audit_record::{HostAuditEvent, HostAuditKind};

const LOG: &str = "[security][audit]";

/// Record `event` for the calling tenant's subject.
pub(crate) async fn record_for_tenant(
    config: &Config,
    tenant: &crate::core::runtime::Tenant,
    event: HostAuditEvent,
) -> Result<(), String> {
    let proxy = super::scoped_client::serving_proxy()
        .await
        .ok_or_else(|| NOT_SERVING.to_owned())?;
    record_for_tenant_with(&proxy, config, tenant, event).await
}

/// Record `event` for the calling tenant's subject through `proxy`.
pub(crate) async fn record_for_tenant_with(
    proxy: &tinybus::Proxy,
    config: &Config,
    tenant: &crate::core::runtime::Tenant,
    event: HostAuditEvent,
) -> Result<(), String> {
    let saas = crate::core::runtime::is_saas();
    let subject = super::policy_translation::subject_scope(tenant, &config.workspace_dir, saas)?;
    record_with(proxy, config, subject, event).await
}

/// The error a row gets when no module is serving; the row is reported, not kept.
pub(crate) const NOT_SERVING: &str = "the TinySecurity module is not serving";

/// Record `event` on `subject`'s stream through `proxy`.
pub(crate) async fn record_with(
    proxy: &tinybus::Proxy,
    config: &Config,
    subject: SubjectScope,
    event: HostAuditEvent,
) -> Result<(), String> {
    let stream = AuditStream::open(config, subject.clone()).await?;
    // Held until the module answers: the tail read here is the tail it extends.
    let _ordered = stream.lock().await;
    let generation = super::sandbox::generation(proxy).await?;
    let audit = stream.correlation(generation).await?;
    let request = request(stream.stream_id(), subject, event, audit.clone())?;
    let response: AuditResponse2 = proxy
        .call_confidential(names::methods::RECORD_AUDIT2, (request,))
        .await
        .map_err(|error| {
            tracing::debug!(error = %error, "{LOG} RecordAudit2 refused");
            "TinySecurity RecordAudit2 failed".to_owned()
        })?;
    committed(&response, &audit)
}

fn request(
    stream_id: &str,
    subject: SubjectScope,
    event: HostAuditEvent,
    audit: AuditCorrelation2,
) -> Result<AuditRecordRequest2, String> {
    Ok(AuditRecordRequest2 {
        subject,
        stream_id: stream_id.to_owned(),
        kind: match event.kind {
            HostAuditKind::Execution => AuditKind2::Execution,
            HostAuditKind::PolicyDecision => AuditKind2::PolicyDecision,
        },
        attribution: event.attribution,
        arguments_json: serde_json::to_string(&event.arguments)
            .map_err(|_| "the audit row is not serializable".to_owned())?,
        rule_ids: Vec::new(),
        audit,
    })
}

/// Whether the module durably recorded exactly the event the host asked for.
pub(crate) fn committed(
    response: &AuditResponse2,
    audit: &AuditCorrelation2,
) -> Result<(), String> {
    let receipt = response
        .receipt
        .as_ref()
        .filter(|_| response.committed)
        .ok_or_else(|| {
            format!(
                "TinySecurity did not commit the audit row ({:?})",
                response.health
            )
        })?;
    let exact = receipt.new_tail == audit.expected_tail.saturating_add(1)
        && receipt.events.len() == 1
        && receipt.events[0].event_id == audit.event_id;
    if exact {
        Ok(())
    } else {
        Err("TinySecurity committed another audit event than the row".into())
    }
}

#[cfg(test)]
#[path = "security_audit_tests.rs"]
mod tests;
