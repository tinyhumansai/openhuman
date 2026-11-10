use super::*;

pub(super) fn record(quote_id: &str, created_at_ms: u64, status: LedgerStatus) -> LedgerRecord {
    LedgerRecord {
        id: format!("id-{quote_id}-{created_at_ms}"),
        quote_id: quote_id.into(),
        created_at_ms,
        updated_at_ms: created_at_ms,
        origin: "automation".into(),
        origin_class: "TrustedAutomation(Background)".into(),
        policy_applied: true,
        chain: "solana".into(),
        evm_network: None,
        asset_symbol: "USDC".into(),
        token_address: None,
        to_address: "Recipient1111111111111111111111111111111111".into(),
        amount_raw: "1000000".into(),
        status,
        transaction_hash: None,
        error: None,
    }
}

#[test]
fn ledger_records_survive_reopening_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    {
        let ledger = SpendLedger::open(dir.path()).unwrap();
        ledger
            .put(&record("q-1", 1_000, LedgerStatus::Pending))
            .unwrap();
        let mut done = record("q-2", 2_000, LedgerStatus::Pending);
        ledger.put(&done).unwrap();
        done.status = LedgerStatus::Broadcast;
        done.transaction_hash = Some("sig".into());
        ledger.put(&done).unwrap();
    }

    let reopened = SpendLedger::open(dir.path()).unwrap();
    let records = reopened.records().unwrap();
    assert_eq!(records.len(), 2, "an update replaces its record");
    assert_eq!(records[0].quote_id, "q-1");
    assert_eq!(records[0].status, LedgerStatus::Pending);
    assert_eq!(records[1].status, LedgerStatus::Broadcast);
    assert_eq!(records[1].transaction_hash.as_deref(), Some("sig"));
    assert_eq!(reopened.recent(1).unwrap()[0].quote_id, "q-2");
}

#[test]
fn a_refused_attempt_is_not_an_attempt_that_may_have_moved_funds() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = SpendLedger::open(dir.path()).unwrap();
    ledger
        .put(&record("q-1", 1_000, LedgerStatus::Refused))
        .unwrap();
    assert!(ledger.attempt_for_quote("q-1").unwrap().is_none());
    ledger
        .put(&record("q-1", 2_000, LedgerStatus::InDoubt))
        .unwrap();
    assert_eq!(
        ledger.attempt_for_quote("q-1").unwrap().unwrap().status,
        LedgerStatus::InDoubt
    );
}
