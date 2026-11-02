//! Host-recorded events on TinySecurity's per-subject audit stream.

use std::time::Duration;

use tinysecurity_bus::{
    names, AuditCorrelation2, AuditKind2, AuditRecordRequest2, AuditResponse2, PolicyInfo,
    SubjectScope,
};

use crate::config::Config;
use crate::core::runtime::Tenant;
use crate::security::approval::native::audit_stream::AuditStream;
use crate::security::audit_record::{HostAuditEvent, HostAuditKind};

const CALL_TIMEOUT: Duration = Duration::from_secs(25);

/// Record a host-observed row for the caller's registered tenant.
pub(crate) async fn record_for_tenant(
    config: &Config,
    tenant: &Tenant,
    event: HostAuditEvent,
) -> Result<(), String> {
    let object = super::approval_object(config, CALL_TIMEOUT).await?;
    let subject = super::policy_translation::subject_scope(
        tenant,
        &config.workspace_dir,
        crate::core::runtime::is_saas(),
    )?;
    record_with(&object, config, subject, event).await
}

/// Record a host-observed row on the supplied TinySecurity object.
pub(crate) async fn record_with(
    object: &super::SecurityProxy,
    config: &Config,
    subject: SubjectScope,
    event: HostAuditEvent,
) -> Result<(), String> {
    let stream = AuditStream::open(config, subject.clone()).await?;
    let _ordered = stream.lock().await;
    let policy: PolicyInfo = object
        .call_confidential(names::methods::POLICY_INFO, ())
        .await
        .map_err(|_| "TinySecurity PolicyInfo failed".to_owned())?;
    let correlation = stream.correlation(policy.generation).await?;
    let request = request(stream.stream_id(), subject, event, correlation.clone())?;
    let response: AuditResponse2 = object
        .call_confidential(names::methods::RECORD_AUDIT2, (request,))
        .await
        .map_err(|_| "TinySecurity RecordAudit2 failed".to_owned())?;
    committed(&response, &correlation)
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

fn committed(response: &AuditResponse2, audit: &AuditCorrelation2) -> Result<(), String> {
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
    if receipt.new_tail == audit.expected_tail.saturating_add(1)
        && receipt.events.len() == 1
        && receipt.events[0].event_id == audit.event_id
    {
        Ok(())
    } else {
        Err("TinySecurity committed another audit event than the row".into())
    }
}

/// Build one subject scope for tests and adjacent host adapters.
#[cfg(test)]
pub(crate) fn subject(user: &str, workspace: &str, agent: &str) -> SubjectScope {
    SubjectScope {
        user: user.into(),
        workspace: workspace.into(),
        agent: agent.into(),
    }
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
