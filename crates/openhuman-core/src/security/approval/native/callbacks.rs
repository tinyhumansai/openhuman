//! The module's callbacks into the host, as plain async functions.
//!
//! `modules/security_host.rs` serves them on the shared callback object, after
//! checking that the caller is the loaded module's own bus connection. Each
//! function here answers for exactly the subject the module names, by the handle
//! the module itself returned when the subject was registered; an unknown handle
//! is refused, so the module can never reach another subject's rows.

use serde_json::Value;
use tinysecurity_bus::{
    ApprovalCasRequest2, ApprovalCasResponse2, ApprovalChoice2, ApprovalDeliveryRequest2,
    ApprovalDeliveryResponse2, ApprovalLoadRequest2, ApprovalLoadResponse2, AuditSinkRequest2,
    AuditSinkResponse2, HumanDecisionReceipt2, VerifyHumanRequest2,
};

use super::super::engine::Delivery;
use super::scopes::{by_handle, by_stream};
use super::state::{host, RequestEntry};
use super::ticket_sha256;

const LOG: &str = "[approval::native]";

/// `ApprovalStore2.Load`: the latest rows of one subject.
pub(crate) async fn load(request: ApprovalLoadRequest2) -> Result<ApprovalLoadResponse2, String> {
    let scope = by_handle(&request.scope)
        .await
        .ok_or("approval store: unknown subject")?;
    let records = scope.rows.load(&request.scope).await?;
    tracing::trace!(records = records.len(), "{LOG} store load");
    Ok(ApprovalLoadResponse2 {
        scope: request.scope,
        records,
    })
}

/// `ApprovalStore2.Cas`: one compare-and-swap of a row.
pub(crate) async fn cas(request: ApprovalCasRequest2) -> Result<ApprovalCasResponse2, String> {
    let scope = by_handle(&request.scope)
        .await
        .ok_or("approval store: unknown subject")?;
    let receipt = scope.rows.cas(&request).await?;
    tracing::trace!(
        version = receipt.resulting_version,
        "{LOG} store cas committed"
    );
    Ok(receipt)
}

/// `AuditSink2.Append`: one ordered audit batch of one subject's stream.
pub(crate) async fn append(request: AuditSinkRequest2) -> Result<AuditSinkResponse2, String> {
    // The batch names its subject only by the module's identifier hash, so the
    // stream the host assigned to the subject is what identifies it; the first
    // batch fixes how the stream's subject is spelled and later ones must repeat it.
    let scope = by_stream(&request.stream_id)
        .await
        .ok_or("audit sink: unknown stream")?;
    if !scope.audit_scope_matches(&request.scope) {
        tracing::warn!("{LOG} audit batch names another subject than its stream's");
        return Err("audit sink: the stream belongs to another subject".into());
    }
    let receipt = scope.rows.append(&request).await?;
    tracing::trace!(new_tail = receipt.new_tail, "{LOG} audit batch committed");
    Ok(receipt)
}

/// `ApprovalPrompt2.Prompt`: show a request the module made durable.
///
/// The call is found by the correlation the host gave it; a request nobody here
/// asked for (another process's pending row, a replay after a restart) has no
/// surface to show it on and is refused, which denies it.
pub(crate) async fn prompt(
    request: ApprovalDeliveryRequest2,
) -> Result<ApprovalDeliveryResponse2, String> {
    let row = request.row;
    let call = host()
        .call(&row.binding.caller.call_id)
        .ok_or("approval prompt: no call is waiting for this request")?;
    if call.scope.handle() != Some(&row.binding.scope) || call.request.call.tool != row.tool_name {
        tracing::warn!("{LOG} prompt for a request that is not this call's");
        return Err("approval prompt: the request is not the waiting call's".into());
    }
    host().insert_request(
        &row.request_id,
        RequestEntry::new(
            std::sync::Arc::clone(&call.scope),
            call.request.clone(),
            Some(row.clone()),
        ),
    );
    let delivery = Delivery {
        call_id: row.binding.caller.call_id.clone(),
        request_id: row.request_id.clone(),
        tool_name: row.tool_name.clone(),
        summary: row.summary.clone(),
        args_redacted: serde_json::from_str::<Value>(&row.args_redacted).unwrap_or(Value::Null),
        created_at_ms: row.created_at_ms,
        expires_at_ms: row.expires_at_ms,
    };
    call.prompt.deliver(delivery).await?;
    Ok(ApprovalDeliveryResponse2 {
        request_id: row.request_id,
        nonce: row.nonce,
        delivered: true,
    })
}

/// `ApprovalPrompt2.VerifyHuman`: bind a human ticket to the exact pending row.
///
/// A ticket exists only because the host authenticated the person who decided
/// (the RPC that carried the decision); the module is told which request, which
/// choice and who. The row the ticket is verified against must be, byte for
/// byte, the row that was shown to that person.
pub(crate) async fn verify_human(
    request: VerifyHumanRequest2,
) -> Result<HumanDecisionReceipt2, String> {
    let (request_id, choice, actor) = host()
        .ticket(&request.decision_ticket)
        .ok_or("human verification: the ticket is unknown or expired")?;
    let known = host()
        .request(&request.row.request_id)
        .ok_or("human verification: the request is not pending in this process")?;
    if request_id != request.row.request_id || known.delivered.as_ref() != Some(&request.row) {
        tracing::warn!("{LOG} human ticket does not match the pending row");
        return Err("human verification: the ticket is not for this request".into());
    }
    if !matches!(choice, ApprovalChoice2::Once | ApprovalChoice2::Deny) {
        return Err("human verification: the choice is not offered".into());
    }
    Ok(HumanDecisionReceipt2 {
        ticket_sha256: ticket_sha256(&request.decision_ticket),
        row: request.row,
        actor,
        choice,
    })
}

#[cfg(test)]
#[path = "callbacks_tests.rs"]
mod tests;
