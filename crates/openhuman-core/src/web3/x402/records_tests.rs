use super::*;
use crate::security::approval::{ApprovalChatContext, APPROVAL_CHAT_CONTEXT};
use crate::web3::wallet::test_support::TEST_LOCK;
use tinywallet_x402::ledger::{self, PaymentStatus};

const SESSION: &str = "x402-boot-session";

fn chat_ctx(thread: &str) -> ApprovalChatContext {
    ApprovalChatContext {
        thread_id: thread.to_string(),
        client_id: "client".to_string(),
        request_id: None,
    }
}

/// A payment as `handle_402_and_pay` hands it back: 2500 atomic USDC held on
/// the process-wide ledger.
fn payment() -> X402PaymentResult {
    X402PaymentResult {
        header_value: "header".into(),
        amount_atomic: 2_500,
        asset: "usdc-mint".into(),
        recipient: "recipient".into(),
        network: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp".into(),
        url: "https://x402.example.test/thing".into(),
        authorization_valid_until: None,
        solana_last_valid_block_height: None,
        reservation: ledger::reserve(2_500).unwrap().unwrap(),
    }
}

fn init(temp: &tempfile::TempDir) {
    super::super::init_ledger(temp.path(), SESSION);
}

/// The fallback path's pending record carries the ledger's session, so the
/// payment counts toward `session_total` once it settles, and the chat thread.
#[tokio::test]
async fn a_fallback_payment_counts_in_the_session_total_and_names_its_thread() {
    let _lock = TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    init(&temp);
    let payment = payment();

    let record = APPROVAL_CHAT_CONTEXT
        .scope(chat_ctx("thread-a"), async { pending_record(&payment) })
        .await;

    assert_eq!(record.status, PaymentStatus::Pending);
    assert_eq!(record.session_id, SESSION);
    assert_eq!(record.thread_id.as_deref(), Some("thread-a"));
    assert_eq!(record.amount_atomic, 2_500);
    assert_eq!(record.amount_display, "0.002500 USDC");
    assert_eq!(record.url, payment.url);

    let mut settled = record.clone();
    settled.status = PaymentStatus::Settled;
    payment.reservation.commit(settled);
    let summary = ledger::with_ledger(|l| l.summary()).unwrap();
    assert_eq!(summary.session_total_atomic, 2_500);
    assert_eq!(summary.session_count, 1);
}

#[tokio::test]
async fn a_payment_outside_a_chat_turn_has_no_thread_but_the_same_session() {
    let _lock = TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    init(&temp);

    let record = pending_record(&payment());

    assert_eq!(record.thread_id, None);
    assert_eq!(record.session_id, SESSION);
}

#[tokio::test]
async fn every_payment_gets_its_own_record_id() {
    let _lock = TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    init(&temp);

    assert_ne!(pending_record(&payment()).id, pending_record(&payment()).id);
}
