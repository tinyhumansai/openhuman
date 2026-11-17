//! Progressive output against the linked TinyChannels module and a mocked
//! Telegram endpoint: target resolution, opening, feeding and finishing.

use super::super::native_media::flow_tests::{envelope, runtime_context};
use super::*;
use crate::channels::runtime::test_support::linked_telegram_runtime;
use tinychannels_bus::module_progressive::ProgressiveResult;

#[tokio::test]
async fn native_target_resolves_only_a_configured_account_of_the_envelope_provider() {
    let linked = linked_telegram_runtime().await;
    let ctx = runtime_context(linked.module_objects());

    let configured = native_target(&ctx, &envelope(Vec::new())).unwrap();
    assert!(configured.is_some(), "the telegram account is configured");

    let mut wrong_provider = envelope(Vec::new());
    wrong_provider.channel.account_id = Some("slack:work".into());
    let err = native_target(&ctx, &wrong_provider)
        .err()
        .expect("provider mismatch");
    assert!(
        err.to_string().contains("no configured account identity"),
        "{err}"
    );

    let mut no_account = envelope(Vec::new());
    no_account.channel.account_id = None;
    assert!(native_target(&ctx, &no_account).is_err());

    let mut unconfigured = envelope(Vec::new());
    unconfigured.channel.account_id = Some("telegram:other".into());
    let err = native_target(&ctx, &unconfigured)
        .err()
        .expect("unknown account");
    assert!(err.to_string().contains("not configured"), "{err}");
}

#[tokio::test]
async fn progress_is_streamed_and_the_final_reply_is_delivered_to_the_provider() {
    let linked = linked_telegram_runtime().await;
    let ctx = runtime_context(linked.module_objects());
    let target = native_target(&ctx, &envelope(Vec::new())).unwrap().unwrap();
    let reply = Some(
        target
            .open("turn-progress-1")
            .await
            .expect("module opens the turn"),
    );

    let (tx, rx) = mpsc::channel(8);
    let worker = spawn_native_progress(rx, reply.clone().unwrap());
    tx.send(AgentProgress::TextDelta {
        delta: "working on it".into(),
        iteration: 1,
    })
    .await
    .unwrap();
    // Events with no channel projection are skipped, not fed.
    tx.send(AgentProgress::TextDelta {
        delta: "more".into(),
        iteration: 1,
    })
    .await
    .unwrap();
    drop(tx);
    worker.await.unwrap();

    assert!(finish_native_reply(&reply, "all done").await);
    let paths = linked.provider_request_paths().await;
    assert!(
        paths
            .iter()
            .any(|p| p.ends_with("/sendMessage") || p.ends_with("/editMessageText")),
        "the provider received the output: {paths:?}"
    );
}

#[tokio::test]
async fn finishing_a_closed_reply_reports_failure_and_missing_replies_are_noops() {
    let linked = linked_telegram_runtime().await;
    let ctx = runtime_context(linked.module_objects());
    let target = native_target(&ctx, &envelope(Vec::new())).unwrap().unwrap();
    let reply = Some(target.open("turn-progress-2").await.unwrap());

    close_native_reply(&reply).await;
    assert!(!finish_native_reply(&reply, "too late").await);
    // Closing twice is tolerated.
    close_native_reply(&reply).await;

    assert!(!finish_native_reply(&None, "nobody listening").await);
    close_native_reply(&None).await;
}

#[tokio::test]
async fn a_failed_progress_update_closes_the_reply() {
    let linked = linked_telegram_runtime().await;
    let ctx = runtime_context(linked.module_objects());
    let target = native_target(&ctx, &envelope(Vec::new())).unwrap().unwrap();
    let reply = target.open("turn-progress-3").await.unwrap();
    reply.close().await.unwrap();

    let (tx, rx) = mpsc::channel(4);
    let worker = spawn_native_progress(rx, reply.clone());
    tx.send(AgentProgress::TextDelta {
        delta: "into a closed turn".into(),
        iteration: 1,
    })
    .await
    .unwrap();
    drop(tx);
    tokio::time::timeout(std::time::Duration::from_secs(5), worker)
        .await
        .expect("the worker stops after the first failed update")
        .unwrap();
}

#[test]
fn delivery_counts_report_outcomes_and_unsupported_parts() {
    use tinychannels_bus::channel::{
        ChannelSendError, ChannelSendOutcome, ContentPart, MessageReceipt, OutboundContentPart,
        SendErrorKind,
    };
    let ok = ChannelSendOutcome {
        receipt: MessageReceipt::default(),
        error: None,
    };
    let failed = ChannelSendOutcome {
        receipt: MessageReceipt::default(),
        error: Some(ChannelSendError {
            kind: SendErrorKind::Transient,
            ..Default::default()
        }),
    };
    let unsupported = vec![OutboundContentPart::Content {
        part: ContentPart::Text {
            text: "x".into(),
            markdown_dialect: None,
        },
    }];

    let single = ProgressiveResult::DeliveryOutcome {
        outcome: Box::new(ok.clone()),
    };
    assert_eq!(delivery_outcome_count(&single), 1);
    assert_eq!(delivery_unsupported_count(&single), 0);

    let many = ProgressiveResult::DeliveryOutcomes {
        outcomes: vec![ok.clone(), failed.clone()],
    };
    assert_eq!(delivery_outcome_count(&many), 2);
    assert!(!delivery_succeeded(&many));
    assert!(delivery_succeeded(&ProgressiveResult::DeliveryOutcomes {
        outcomes: vec![ok.clone()]
    }));
    assert!(!delivery_succeeded(&ProgressiveResult::DeliveryOutcome {
        outcome: Box::new(failed)
    }));

    let batch = ProgressiveResult::DeliveryBatch {
        outcomes: vec![ok],
        unsupported: unsupported.clone(),
        failure: None,
        hosted_accepted: false,
    };
    assert_eq!(delivery_outcome_count(&batch), 1);
    assert_eq!(delivery_unsupported_count(&batch), 1);
    assert!(
        !delivery_succeeded(&batch),
        "unsupported parts are not a complete delivery"
    );

    let unsupported_only = ProgressiveResult::DeliveryUnsupported { parts: unsupported };
    assert_eq!(delivery_unsupported_count(&unsupported_only), 1);
    assert_eq!(delivery_outcome_count(&unsupported_only), 0);
    assert!(delivery_succeeded(
        &ProgressiveResult::DeliveryUnsupported { parts: Vec::new() }
    ));
    assert!(delivery_succeeded(&ProgressiveResult::Updated));
}
