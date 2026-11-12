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

fn agent_context(
    parent: &std::sync::Arc<crate::core::runtime::CoreContext>,
    id: &str,
) -> std::sync::Arc<crate::core::runtime::CoreContext> {
    use crate::core::runtime::{ContextOverlay, DomainSet};
    use crate::tools::toolpacks::ToolGroups;
    parent.derive_with(
        ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::kernel(),
            ToolGroups::none(),
        )
        .session_agent(id),
    )
}

#[tokio::test]
async fn agents_sharing_a_workspace_share_one_ledger() {
    use crate::core::runtime::{CoreContext, DomainSet};
    let dir = tempfile::tempdir().unwrap();
    let root = CoreContext::for_test(DomainSet::full(), Some(dir.path().to_path_buf()));
    let alpha = agent_context(&root, "ledger-agent-alpha");
    let beta = agent_context(&root, "ledger-agent-beta");

    let path = dir.path().to_path_buf();
    CoreContext::scope(alpha, async {
        SpendLedger::open(&path)
            .unwrap()
            .put(&record("q-alpha", 1_000, LedgerStatus::Broadcast))
            .unwrap();
    })
    .await;
    let seen = CoreContext::scope(beta, async {
        SpendLedger::open(&path).unwrap().records().unwrap()
    })
    .await;
    assert_eq!(
        seen.iter().map(|r| r.quote_id.as_str()).collect::<Vec<_>>(),
        ["q-alpha"],
        "a second agent spends from the same wallet and must see the first agent's spend"
    );
}

#[test]
fn a_reservation_that_lost_a_race_is_withdrawn_and_re_decided() {
    let dir = tempfile::tempdir().unwrap();
    let ours = SpendLedger::open(dir.path()).unwrap();
    let other_core = SpendLedger::open(dir.path()).unwrap();

    let mut calls = 0;
    let outcome = ours
        .reserve(|records| {
            calls += 1;
            if calls == 1 {
                // Another core reserves between our read and our claim.
                assert!(records.is_empty());
                other_core
                    .reserve(|_| Ok::<_, ()>(record("q-other", 1_000, LedgerStatus::Pending)))
                    .unwrap()
                    .unwrap();
            } else {
                assert_eq!(records.len(), 1, "the retry sees the other core's record");
                return Err("over the cap");
            }
            Ok(record("q-ours", 2_000, LedgerStatus::Pending))
        })
        .unwrap();

    assert_eq!(calls, 2);
    assert_eq!(outcome.unwrap_err(), "over the cap");
    let remaining = ours.records().unwrap();
    assert_eq!(
        remaining.len(),
        1,
        "our withdrawn reservation must not linger"
    );
    assert_eq!(remaining[0].quote_id, "q-other");
}
