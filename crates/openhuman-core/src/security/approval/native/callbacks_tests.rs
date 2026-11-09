use super::*;
use crate::security::approval::engine::PromptSurface;
use crate::security::approval::native::rows::Rows;
use crate::security::approval::native::scopes::ScopeEntry;
use crate::security::approval::native::state::Ticket;
use std::sync::{Arc, Mutex};
use tinysecurity_bus::{
    ApprovalBinding2, ApprovalLifecycle2, ApprovalRow2, AuditCorrelation2, CallerContext, Check,
    EvaluateRequest, Origin, SubjectScope, ToolCall,
};
use tinystoragedrivers::{Scope, StorageBackend};

#[derive(Default)]
struct Surface {
    shown: Mutex<Vec<Delivery>>,
    refuse: bool,
}

#[async_trait::async_trait]
impl PromptSurface for Surface {
    async fn deliver(&self, delivery: Delivery) -> Result<(), String> {
        if self.refuse {
            return Err("nobody to ask".into());
        }
        self.shown.lock().unwrap().push(delivery);
        Ok(())
    }
}

fn subject(name: &str) -> SubjectScope {
    SubjectScope {
        user: "local".into(),
        workspace: format!("/callbacks/{name}"),
        agent: "agent".into(),
    }
}

fn handle(name: &str) -> SubjectScope {
    SubjectScope {
        user: format!("{name}-user").repeat(8),
        workspace: format!("{name}-workspace").repeat(8),
        agent: format!("{name}-agent").repeat(8),
    }
}

async fn bound(name: &str) -> Arc<ScopeEntry> {
    let storage = tinystoragedrivers_testkit::memory_backend();
    let scoped = storage.for_scope(&Scope::new("local").unwrap()).unwrap();
    let entry = Arc::new(ScopeEntry::new(
        subject(name),
        uuid::Uuid::new_v4().simple().to_string(),
        Rows::over(Arc::clone(scoped.documents())),
    ));
    entry.bind(handle(name)).await;
    entry
}

#[tokio::test]
async fn native_secret_grant_prompt_uses_the_exact_in_flight_scope_route() {
    let scope = bound("native-secret-grant").await;
    let surface = Arc::new(Surface::default());
    assert!(host().begin_secret_prompt(Arc::clone(&scope), surface.clone()));

    let pending = row(
        "native-secret-grant",
        "a",
        "native-keyed-call-correlation",
        "native_secret_grant",
    );
    let request_id = pending.request_id.clone();
    let response = prompt(delivery_for(pending)).await.unwrap();

    assert!(response.delivered);
    assert_eq!(response.request_id, request_id);
    assert_eq!(surface.shown.lock().unwrap().len(), 1);
    let remembered = host().request(&request_id).expect("pending row remembered");
    assert_eq!(remembered.scope.scope_id, scope.scope_id);
    assert_eq!(
        remembered.delivered.unwrap().tool_name,
        "native_secret_grant"
    );
}

fn request(tool: &str, call_id: &str) -> EvaluateRequest {
    EvaluateRequest {
        caller: CallerContext {
            agent: "agent".into(),
            call_id: call_id.into(),
            ..Default::default()
        },
        call: ToolCall {
            tool: tool.into(),
            arguments_json: "{}".into(),
            checks: vec![Check::HostTool {}],
        },
    }
}

fn row(name: &str, id: &str, call_id: &str, tool: &str) -> ApprovalRow2 {
    ApprovalRow2 {
        request_id: id.repeat(32)[..32].to_owned(),
        version: 1,
        binding: ApprovalBinding2 {
            binding_version: 1,
            scope: handle(name),
            policy_ref: "p".repeat(64),
            generation: 1,
            call_sha256: "c".repeat(64),
            content_sha256: "d".repeat(64),
            origin: Origin::Operator,
            caller: CallerContext {
                agent: "agent".into(),
                call_id: call_id.into(),
                ..Default::default()
            },
        },
        host_instance: "e".repeat(32),
        nonce: "f".repeat(32),
        tool_name: tool.into(),
        session_id: None,
        audit_error: None,
        external_effect: true,
        args_redacted: r#"{"to":"x"}"#.into(),
        summary: "send".into(),
        created_at_ms: 1_000,
        expires_at_ms: 601_000,
        state: ApprovalLifecycle2::Pending,
        choice: None,
        attribution: None,
        decision_ticket_sha256: None,
        outcome: None,
        reservation_id: None,
        legacy_history_json: None,
        judge: None,
        audit: AuditCorrelation2 {
            event_id: "a".repeat(32),
            expected_tail: 0,
            generation: 1,
        },
    }
}

fn delivery_for(row: ApprovalRow2) -> ApprovalDeliveryRequest2 {
    ApprovalDeliveryRequest2 { row }
}

fn audit_event(sequence: u64) -> tinysecurity_bus::AuditEvent2 {
    tinysecurity_bus::AuditEvent2 {
        sequence,
        event_id: format!("{sequence:032x}"),
        generation: 1,
        kind: tinysecurity_bus::AuditKind2::Approval,
        attribution: "pending".into(),
        arguments_json: "{}".into(),
        rule_ids: vec![],
    }
}

/// The subject as the module's audit batches spell it: an identifier hash per part.
fn audit_scope(name: &str) -> SubjectScope {
    SubjectScope {
        user: format!("{:08x}", name.len()),
        workspace: "11111111".into(),
        agent: "22222222".into(),
    }
}

#[tokio::test]
async fn the_store_answers_only_for_a_handle_the_module_registered() {
    let load = ApprovalLoadRequest2 {
        scope: handle("unregistered-store"),
    };
    assert!(super::load(load).await.is_err());
    let cas = ApprovalCasRequest2 {
        scope: handle("unregistered-store"),
        record_id: "1".repeat(32),
        expected_version: None,
        next_record: tinysecurity_bus::ApprovalRecord2 {
            record_id: "1".repeat(32),
            version: 1,
            policy_id: "p".into(),
            generation: 1,
            fingerprint: "2".repeat(32),
            record_json: "{}".into(),
        },
        mutation_id: "3".repeat(32),
    };
    assert!(super::cas(cas).await.is_err());

    let _entry = bound("store-known").await;
    let loaded = super::load(ApprovalLoadRequest2 {
        scope: handle("store-known"),
    })
    .await
    .unwrap();
    assert!(loaded.records.is_empty());
    assert_eq!(loaded.scope, handle("store-known"));
}

#[tokio::test]
async fn the_sink_names_its_subject_by_the_stream_the_host_assigned() {
    let entry = bound("sink-known").await;
    // A stream nobody registered (another subject, or nothing) is refused.
    let unknown = AuditSinkRequest2 {
        scope: audit_scope("sink-known"),
        stream_id: "9".repeat(32),
        expected_tail: 0,
        events: vec![audit_event(1)],
    };
    assert!(super::append(unknown).await.is_err());

    // The registered stream commits, and returns the exact receipt.
    let batch = AuditSinkRequest2 {
        scope: audit_scope("sink-known"),
        stream_id: entry.stream_id.clone(),
        expected_tail: 0,
        events: vec![audit_event(1)],
    };
    let receipt = super::append(batch.clone()).await.unwrap();
    batch.verify_receipt(&receipt).unwrap();
    assert_eq!(entry.rows.tail(&entry.stream_id).await.unwrap(), 1);

    // The first batch fixed how the stream's subject is spelled: another subject
    // cannot write into it.
    let foreign = AuditSinkRequest2 {
        scope: audit_scope("someone-else-entirely"),
        expected_tail: 1,
        events: vec![audit_event(2)],
        ..batch
    };
    assert!(super::append(foreign).await.is_err());
    assert_eq!(entry.rows.tail(&entry.stream_id).await.unwrap(), 1);
}

#[tokio::test]
async fn a_prompt_reaches_the_call_that_asked_and_registers_its_request() {
    let scope = bound("prompt-ok").await;
    let surface = Arc::new(Surface::default());
    let call_id = "aaaa0001";
    assert!(host().begin_call(
        call_id,
        crate::security::approval::native::state::CallEntry {
            prompt: surface.clone(),
            scope: Arc::clone(&scope),
            request: request("gmail_send", call_id),
        },
    ));
    let pending = row("prompt-ok", "1", call_id, "gmail_send");
    let response = super::prompt(delivery_for(pending.clone())).await.unwrap();
    assert!(response.delivered);
    assert_eq!(response.request_id, pending.request_id);
    assert_eq!(response.nonce, pending.nonce);

    let shown = surface.shown.lock().unwrap().clone();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].call_id, call_id);
    assert_eq!(shown[0].tool_name, "gmail_send");
    assert_eq!(shown[0].args_redacted["to"], "x");
    let known = host().request(&pending.request_id).expect("remembered");
    assert_eq!(known.delivered, Some(pending));
    host().end_call(call_id);
    host().forget_request(&known.delivered.unwrap().request_id);
}

#[tokio::test]
async fn store_and_prompt_preserve_tinysecurity_redaction_markers() {
    let scope = bound("redacted-prompt").await;
    let call_id = "redacted01";
    let mut pending = row("redacted-prompt", "7", call_id, "gmail_send");
    pending.args_redacted = serde_json::json!({
        "body": "<redacted: string (22 chars)>",
        "api_key": "<redacted: string (22 chars)>",
    })
    .to_string();
    let next_record = tinysecurity_bus::ApprovalRecord2 {
        record_id: pending.request_id.clone(),
        version: pending.version,
        policy_id: pending.binding.policy_ref.clone(),
        generation: pending.binding.generation,
        fingerprint: "9".repeat(32),
        record_json: serde_json::to_string(&pending).unwrap(),
    };
    super::cas(ApprovalCasRequest2 {
        scope: handle("redacted-prompt"),
        record_id: pending.request_id.clone(),
        expected_version: None,
        next_record,
        mutation_id: "8".repeat(32),
    })
    .await
    .unwrap();
    let stored = super::load(ApprovalLoadRequest2 {
        scope: handle("redacted-prompt"),
    })
    .await
    .unwrap();
    let persisted: ApprovalRow2 = serde_json::from_str(&stored.records[0].record_json).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&persisted.args_redacted).unwrap(),
        serde_json::json!({
            "body": "<redacted: string (22 chars)>",
            "api_key": "<redacted: string (22 chars)>",
        })
    );
    assert!(!persisted.args_redacted.contains("not-a-real-token-value"));

    let surface = Arc::new(Surface::default());
    assert!(host().begin_call(
        call_id,
        crate::security::approval::native::state::CallEntry {
            prompt: surface.clone(),
            scope,
            request: request("gmail_send", call_id),
        },
    ));
    super::prompt(delivery_for(pending)).await.unwrap();
    let shown = surface.shown.lock().unwrap();
    assert_eq!(
        shown[0].args_redacted,
        serde_json::json!({
            "body": "<redacted: string (22 chars)>",
            "api_key": "<redacted: string (22 chars)>",
        })
    );
    assert!(!shown[0]
        .args_redacted
        .to_string()
        .contains("not-a-real-token-value"));
    host().end_call(call_id);
}

#[tokio::test]
async fn a_prompt_nobody_asked_for_or_that_is_not_the_calls_is_refused() {
    // No call is waiting under this correlation (another process's request).
    assert!(super::prompt(delivery_for(row(
        "prompt-none",
        "2",
        "bbbb0002",
        "gmail_send"
    )))
    .await
    .is_err());

    let scope = bound("prompt-mismatch").await;
    let surface = Arc::new(Surface::default());
    assert!(host().begin_call(
        "cccc0003",
        crate::security::approval::native::state::CallEntry {
            prompt: surface.clone(),
            scope: Arc::clone(&scope),
            request: request("gmail_send", "cccc0003"),
        },
    ));
    // Another tool than the call asked about.
    assert!(super::prompt(delivery_for(row(
        "prompt-mismatch",
        "3",
        "cccc0003",
        "slack_post"
    )))
    .await
    .is_err());
    // Another subject than the call's.
    assert!(super::prompt(delivery_for(row(
        "someone-else",
        "3",
        "cccc0003",
        "gmail_send"
    )))
    .await
    .is_err());
    assert!(surface.shown.lock().unwrap().is_empty());
    host().end_call("cccc0003");
}

#[tokio::test]
async fn a_surface_that_cannot_reach_a_person_refuses_the_delivery() {
    let scope = bound("prompt-refused").await;
    let surface = Arc::new(Surface {
        refuse: true,
        ..Surface::default()
    });
    assert!(host().begin_call(
        "dddd0004",
        crate::security::approval::native::state::CallEntry {
            prompt: surface,
            scope,
            request: request("gmail_send", "dddd0004"),
        },
    ));
    assert!(
        super::prompt(delivery_for(row(
            "prompt-refused",
            "4",
            "dddd0004",
            "gmail_send"
        )))
        .await
        .is_err(),
        "an unreachable human denies the call"
    );
    host().end_call("dddd0004");
}

async fn pending_request(name: &str, id: &str, call_id: &str) -> (ApprovalRow2, Arc<ScopeEntry>) {
    let scope = bound(name).await;
    let pending = row(name, id, call_id, "gmail_send");
    host().insert_request(
        &pending.request_id,
        crate::security::approval::native::state::RequestEntry::new(
            Arc::clone(&scope),
            request("gmail_send", call_id),
            Some(pending.clone()),
        ),
    );
    (pending, scope)
}

fn verify(row: &ApprovalRow2, ticket: &str) -> VerifyHumanRequest2 {
    VerifyHumanRequest2 {
        row: row.clone(),
        decision_ticket: ticket.into(),
    }
}

#[tokio::test]
async fn a_ticket_the_host_issued_for_this_row_yields_a_receipt() {
    let (pending, _scope) = pending_request("verify-ok", "5", "eeee0005").await;
    host().issue(
        "ticket-ok".into(),
        Ticket::new(
            pending.request_id.clone(),
            ApprovalChoice2::Once,
            "operator".into(),
        ),
    );
    let receipt = super::verify_human(verify(&pending, "ticket-ok"))
        .await
        .unwrap();
    assert_eq!(receipt.row, pending);
    assert_eq!(receipt.actor, "operator");
    assert_eq!(receipt.choice, ApprovalChoice2::Once);
    assert_eq!(receipt.ticket_sha256, ticket_sha256("ticket-ok"));
    assert_eq!(receipt.ticket_sha256.len(), 64);
    // The module may verify the same ticket again when it retries.
    assert!(super::verify_human(verify(&pending, "ticket-ok"))
        .await
        .is_ok());
    host().revoke("ticket-ok");
    assert!(
        super::verify_human(verify(&pending, "ticket-ok"))
            .await
            .is_err(),
        "a revoked ticket verifies nothing"
    );
}

#[tokio::test]
async fn a_forged_ticket_or_another_row_is_never_verified() {
    let (pending, _scope) = pending_request("verify-forged", "6", "ffff0006").await;
    // A ticket the host never issued, which is all a model could supply.
    assert!(super::verify_human(verify(&pending, "model-claimed-human"))
        .await
        .is_err());

    // A real ticket, but for another request.
    host().issue(
        "ticket-other".into(),
        Ticket::new("7".repeat(32), ApprovalChoice2::Once, "operator".into()),
    );
    assert!(super::verify_human(verify(&pending, "ticket-other"))
        .await
        .is_err());

    // A real ticket for this request, presented with a row that differs from what was shown.
    host().issue(
        "ticket-altered".into(),
        Ticket::new(
            pending.request_id.clone(),
            ApprovalChoice2::Once,
            "operator".into(),
        ),
    );
    let mut altered = pending.clone();
    altered.summary = "a different action".into();
    assert!(super::verify_human(verify(&altered, "ticket-altered"))
        .await
        .is_err());

    // A request this process never showed.
    let stranger = row("verify-forged", "8", "9999aaaa", "gmail_send");
    host().issue(
        "ticket-stranger".into(),
        Ticket::new(
            stranger.request_id.clone(),
            ApprovalChoice2::Once,
            "operator".into(),
        ),
    );
    assert!(super::verify_human(verify(&stranger, "ticket-stranger"))
        .await
        .is_err());
    for ticket in ["ticket-other", "ticket-altered", "ticket-stranger"] {
        host().revoke(ticket);
    }
}

#[tokio::test]
async fn only_the_choices_the_host_offers_are_verified() {
    let (pending, _scope) = pending_request("verify-choice", "9", "12340009").await;
    host().issue(
        "ticket-tool".into(),
        Ticket::new(
            pending.request_id.clone(),
            ApprovalChoice2::Tool,
            "operator".into(),
        ),
    );
    assert!(super::verify_human(verify(&pending, "ticket-tool"))
        .await
        .is_err());
    host().revoke("ticket-tool");
    host().issue(
        "ticket-deny".into(),
        Ticket::new(
            pending.request_id.clone(),
            ApprovalChoice2::Deny,
            "operator".into(),
        ),
    );
    assert_eq!(
        super::verify_human(verify(&pending, "ticket-deny"))
            .await
            .unwrap()
            .choice,
        ApprovalChoice2::Deny
    );
    host().revoke("ticket-deny");
}
