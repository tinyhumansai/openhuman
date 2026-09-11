use super::*;

fn test_budget() -> SpendingBudget {
    SpendingBudget {
        per_request_max_atomic: 500_000,
        daily_max_atomic: 2_000_000,
        monthly_max_atomic: 10_000_000,
    }
}

const TEST_SESSION: &str = "test";

fn make_record(amount: u64, status: PaymentStatus) -> PaymentRecord {
    PaymentRecord {
        id: uuid::Uuid::new_v4().to_string(),
        url: "https://api.example.com/data".into(),
        asset: "USDC".into(),
        amount_atomic: amount,
        amount_display: format!("{:.6} USDC", amount as f64 / 1_000_000.0),
        recipient: "RecipientPubkey".into(),
        network: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp".into(),
        tx_signature: Some("sig123".into()),
        status,
        timestamp: Utc::now(),
        session_id: TEST_SESSION.into(),
    }
}

#[test]
fn budget_check_allows_within_limits() {
    let ledger = PaymentLedger {
        records: vec![],
        file_path: PathBuf::from("/tmp/test-x402.jsonl"),
        budget: test_budget(),
        session_id: "test".into(),
    };
    assert_eq!(ledger.check_budget(100_000), BudgetCheck::Allowed);
}

#[test]
fn budget_check_rejects_over_per_request() {
    let ledger = PaymentLedger {
        records: vec![],
        file_path: PathBuf::from("/tmp/test-x402.jsonl"),
        budget: test_budget(),
        session_id: "test".into(),
    };
    assert_eq!(
        ledger.check_budget(600_000),
        BudgetCheck::ExceedsPerRequest {
            requested: 600_000,
            cap: 500_000
        }
    );
}

#[test]
fn budget_check_rejects_over_daily() {
    let mut ledger = PaymentLedger {
        records: vec![],
        file_path: PathBuf::from("/tmp/test-x402.jsonl"),
        budget: test_budget(),
        session_id: "test".into(),
    };
    ledger
        .records
        .push(make_record(1_800_000, PaymentStatus::Settled));
    assert_eq!(
        ledger.check_budget(400_000),
        BudgetCheck::ExceedsDailyBudget {
            current: 1_800_000,
            cap: 2_000_000
        }
    );
}

#[test]
fn budget_check_ignores_failed_payments() {
    let mut ledger = PaymentLedger {
        records: vec![],
        file_path: PathBuf::from("/tmp/test-x402.jsonl"),
        budget: test_budget(),
        session_id: "test".into(),
    };
    ledger
        .records
        .push(make_record(1_800_000, PaymentStatus::Failed));
    assert_eq!(ledger.check_budget(400_000), BudgetCheck::Allowed);
}

#[test]
fn summary_aggregates_correctly() {
    let mut ledger = PaymentLedger {
        records: vec![],
        file_path: PathBuf::from("/tmp/test-x402.jsonl"),
        budget: test_budget(),
        session_id: "test".into(),
    };
    ledger
        .records
        .push(make_record(100_000, PaymentStatus::Settled));
    ledger
        .records
        .push(make_record(200_000, PaymentStatus::Settled));
    ledger
        .records
        .push(make_record(50_000, PaymentStatus::Failed));

    let summary = ledger.summary();
    assert_eq!(summary.session_total_atomic, 300_000);
    assert_eq!(summary.session_count, 2);
}
