use super::*;

#[test]
fn native_text_progress_keeps_iteration_and_content() {
    let event = output_event(AgentProgress::TextDelta {
        delta: "partial answer".into(),
        iteration: 3,
    })
    .expect("text progress is projected");
    assert!(matches!(
        event,
        ChannelOutputEvent::TextDelta { text, iteration: Some(3) }
            if text == "partial answer"
    ));
}

#[test]
fn native_tool_progress_keeps_correlation_and_display_metadata() {
    let event = output_event(AgentProgress::ToolCallStarted {
        call_id: "call-1".into(),
        tool_name: "file_read".into(),
        arguments: serde_json::json!({"path":"README.md"}),
        iteration: 2,
        display_label: Some("Reading file".into()),
        display_detail: Some("README.md".into()),
    })
    .expect("tool progress is projected");
    assert!(matches!(
        event,
        ChannelOutputEvent::ToolCallStarted {
            call_id,
            tool_name,
            arguments,
            iteration: 2,
            display_label: Some(label),
            display_detail: Some(detail),
        } if call_id == "call-1"
            && tool_name == "file_read"
            && arguments["path"] == "README.md"
            && label == "Reading file"
            && detail == "README.md"
    ));
}

#[test]
fn native_tool_completion_preserves_failure_and_shared_payload_caps() {
    let failure = tinytools::status::classify("[not-found] missing file", false);
    let event = output_event(AgentProgress::ToolCallCompleted {
        call_id: "call-failed".into(),
        tool_name: "file_read".into(),
        success: false,
        output_chars: 4,
        output: "x".repeat(tinychannels_bus::web::MAX_WEB_TOOL_PAYLOAD_BYTES + 32),
        arguments: Some(serde_json::json!({
            "path": "y".repeat(tinychannels_bus::web::MAX_WEB_TOOL_PAYLOAD_BYTES)
        })),
        elapsed_ms: 12,
        iteration: 4,
        failure: Some(failure.clone()),
        display_label: None,
        display_detail: None,
        structured: Some(serde_json::json!({"kind": "read"})),
    })
    .expect("tool completion is projected");
    let ChannelOutputEvent::ToolCallCompleted {
        output,
        arguments,
        failure: summary,
        success,
        structured,
        ..
    } = event
    else {
        panic!("tool completion")
    };
    assert!(output.len() <= tinychannels_bus::web::MAX_WEB_TOOL_PAYLOAD_BYTES);
    assert!(output.contains("truncated"));
    let arguments = arguments.expect("arguments");
    assert!(arguments.to_string().contains("truncated"));
    let summary = summary.expect("classified failure");
    assert_eq!(summary.cause_plain, failure.cause_plain);
    assert_eq!(summary.recoverable, failure.recoverable);
    assert!(!summary.class.is_empty());
    assert!(!summary.category.is_empty());
    assert!(!success);
    assert_eq!(structured, Some(serde_json::json!({"kind": "read"})));
}

#[test]
fn progressive_delivery_status_rejects_partial_or_unsupported_terminal_results() {
    use tinychannels_bus::module_progressive::{ProgressiveOutputFailure, ProgressiveResult};

    let receipt = tinychannels_bus::channel::MessageReceipt {
        primary_platform_message_id: Some("provider-message-9".into()),
        ..Default::default()
    };
    let delivered = ProgressiveResult::DeliveryOutcome {
        outcome: Box::new(tinychannels_bus::channel::ChannelSendOutcome {
            receipt: receipt.clone(),
            error: None,
        }),
    };
    assert!(delivery_succeeded(&delivered));
    assert!(matches!(
        delivered,
        ProgressiveResult::DeliveryOutcome { outcome }
            if outcome.receipt.primary_platform_message_id.as_deref() == Some("provider-message-9")
    ));

    let partial = ProgressiveResult::DeliveryBatch {
        outcomes: vec![tinychannels_bus::channel::ChannelSendOutcome {
            receipt,
            error: Some(tinychannels_bus::channel::ChannelSendError {
                kind: tinychannels_bus::channel::SendErrorKind::Transient,
                sent_before_error: true,
                ..Default::default()
            }),
        }],
        unsupported: Vec::new(),
        failure: Some(ProgressiveOutputFailure::ProviderTransport),
        hosted_accepted: false,
    };
    assert!(!delivery_succeeded(&partial));

    let unsupported = ProgressiveResult::DeliveryUnsupported {
        parts: vec![tinychannels_bus::channel::OutboundContentPart::Content {
            part: tinychannels_bus::channel::ContentPart::Text {
                text: "retained content".into(),
                markdown_dialect: None,
            },
        }],
    };
    assert!(!delivery_succeeded(&unsupported));
}

#[test]
fn progressive_delivery_failure_kinds_name_sanitized_causes_only() {
    use tinychannels_bus::channel::{ChannelSendError, ChannelSendOutcome, SendErrorKind};
    use tinychannels_bus::module_progressive::{ProgressiveOutputFailure, ProgressiveResult};

    fn outcome(kind: Option<SendErrorKind>) -> ChannelSendOutcome {
        ChannelSendOutcome {
            receipt: Default::default(),
            error: kind.map(|kind| ChannelSendError {
                kind,
                message: "provider said something sensitive".into(),
                ..Default::default()
            }),
        }
    }

    let failed_outcome = ProgressiveResult::DeliveryOutcome {
        outcome: Box::new(outcome(Some(SendErrorKind::Transient))),
    };
    assert_eq!(delivery_failure_kinds(&failed_outcome), ["transient"]);

    let failed_outcomes = ProgressiveResult::DeliveryOutcomes {
        outcomes: vec![
            outcome(Some(SendErrorKind::RateLimited)),
            outcome(Some(SendErrorKind::Transient)),
            outcome(Some(SendErrorKind::RateLimited)),
            outcome(None),
        ],
    };
    assert_eq!(
        delivery_failure_kinds(&failed_outcomes),
        ["rate_limited", "transient"]
    );

    let batch = ProgressiveResult::DeliveryBatch {
        outcomes: vec![outcome(Some(SendErrorKind::Forbidden))],
        unsupported: vec![tinychannels_bus::channel::OutboundContentPart::Content {
            part: tinychannels_bus::channel::ContentPart::Text {
                text: "retained content".into(),
                markdown_dialect: None,
            },
        }],
        failure: Some(ProgressiveOutputFailure::ProviderTransport),
        hosted_accepted: false,
    };
    assert_eq!(
        delivery_failure_kinds(&batch),
        ["forbidden", "unsupported_parts", "provider_transport"]
    );

    let unsupported = ProgressiveResult::DeliveryUnsupported {
        parts: vec![tinychannels_bus::channel::OutboundContentPart::Content {
            part: tinychannels_bus::channel::ContentPart::Text {
                text: "retained content".into(),
                markdown_dialect: None,
            },
        }],
    };
    assert_eq!(delivery_failure_kinds(&unsupported), ["unsupported_parts"]);

    let clean_outcome = ProgressiveResult::DeliveryOutcome {
        outcome: Box::new(outcome(None)),
    };
    assert!(delivery_failure_kinds(&clean_outcome).is_empty());
    assert!(delivery_failure_kinds(&ProgressiveResult::Updated).is_empty());
}
