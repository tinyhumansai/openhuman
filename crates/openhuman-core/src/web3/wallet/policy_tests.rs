use super::*;
use crate::config::schema::WalletAssetCap;
use crate::web3::wallet::ledger::LedgerStatus;
use std::sync::atomic::{AtomicUsize, Ordering};
use tinywallet_bus::wire::{PreparedKind, PreparedStatus, WalletChain};

const NOW: u64 = 10 * WEEK_MS;
const RECIPIENT: &str = "Recipient1111111111111111111111111111111111";

fn usdc_cap() -> WalletAssetCap {
    WalletAssetCap {
        chain: "solana".into(),
        symbol: "USDC".into(),
        per_tx_max_raw: Some(50_000_000),
        daily_max_raw: Some(100_000_000),
        recipient_weekly_max_raw: Some(50_000_000),
        ..WalletAssetCap::default()
    }
}

fn policy() -> WalletPolicyConfig {
    WalletPolicyConfig {
        automation_enabled: true,
        apply_to_interactive: false,
        assets: vec![usdc_cap()],
    }
}

fn quote(quote_id: &str, symbol: &str, amount: u64, to: &str) -> PreparedTransaction {
    PreparedTransaction {
        quote_id: quote_id.into(),
        kind: PreparedKind::TokenTransfer,
        chain: WalletChain::Solana,
        evm_network: None,
        from_address: "From111111111111111111111111111111111111111".into(),
        to_address: to.into(),
        asset_symbol: symbol.into(),
        amount_raw: amount.to_string(),
        amount_formatted: String::new(),
        receive_symbol: None,
        min_receive_raw: None,
        calldata: None,
        token_address: None,
        estimated_fee_raw: "5000".into(),
        status: PreparedStatus::AwaitingConfirmation,
        created_at_ms: NOW,
        expires_at_ms: NOW + 60_000,
        notes: Vec::new(),
    }
}

fn automation() -> ExecutionOrigin {
    ExecutionOrigin::Automation {
        class: "TrustedAutomation(Background)".into(),
    }
}

fn spent(amount: u64, to: &str, at: u64, status: LedgerStatus) -> LedgerRecord {
    let mut record = new_record(
        &quote("old", "USDC", amount, to),
        &automation(),
        true,
        status,
        at,
    );
    record.quote_id = format!("old-{at}");
    record
}

fn kind(result: Result<bool, PolicyRefusal>) -> RefusalKind {
    result.expect_err("expected a refusal").kind
}

#[test]
fn automation_under_every_cap_is_allowed_and_capped() {
    let q = quote("q", "usdc", 50_000_000, RECIPIENT);
    assert_eq!(evaluate(&policy(), &automation(), &q, &[], NOW), Ok(true));
}

#[test]
fn over_the_per_transaction_cap_is_refused() {
    let q = quote("q", "USDC", 50_000_001, RECIPIENT);
    assert_eq!(
        kind(evaluate(&policy(), &automation(), &q, &[], NOW)),
        RefusalKind::PerTransaction
    );
}

#[test]
fn over_the_rolling_daily_cap_is_refused_and_older_spend_rolls_off() {
    let records = [
        spent(40_000_000, "A", NOW - 1_000, LedgerStatus::Broadcast),
        spent(40_000_000, "B", NOW - 2_000, LedgerStatus::InDoubt),
    ];
    let q = quote("q", "USDC", 30_000_000, "C");
    assert_eq!(
        kind(evaluate(&policy(), &automation(), &q, &records, NOW)),
        RefusalKind::Daily
    );

    // The same spend more than 24 hours ago no longer counts.
    let old = [
        spent(40_000_000, "A", NOW - DAY_MS - 1, LedgerStatus::Broadcast),
        spent(40_000_000, "B", NOW - DAY_MS - 1, LedgerStatus::Broadcast),
    ];
    assert_eq!(evaluate(&policy(), &automation(), &q, &old, NOW), Ok(true));
}

#[test]
fn over_the_per_recipient_weekly_cap_is_refused_while_another_recipient_is_fine() {
    let records = [spent(
        30_000_000,
        RECIPIENT,
        NOW - 3 * DAY_MS,
        LedgerStatus::Broadcast,
    )];
    let q = quote("q", "USDC", 30_000_000, RECIPIENT);
    assert_eq!(
        kind(evaluate(&policy(), &automation(), &q, &records, NOW)),
        RefusalKind::RecipientWeekly
    );
    let other = quote("q", "USDC", 30_000_000, "Other");
    assert_eq!(
        evaluate(&policy(), &automation(), &other, &records, NOW),
        Ok(true)
    );
}

#[test]
fn pending_and_in_doubt_records_count_but_refused_ones_do_not() {
    let pending = [spent(50_000_000, RECIPIENT, NOW - 1, LedgerStatus::Pending)];
    let q = quote("q", "USDC", 1, RECIPIENT);
    assert_eq!(
        kind(evaluate(&policy(), &automation(), &q, &pending, NOW)),
        RefusalKind::RecipientWeekly
    );
    let refused = [spent(50_000_000, RECIPIENT, NOW - 1, LedgerStatus::Refused)];
    assert_eq!(
        evaluate(&policy(), &automation(), &q, &refused, NOW),
        Ok(true)
    );
}

#[test]
fn an_asset_outside_the_allowlist_is_refused_for_automation() {
    let q = quote("q", "BONK", 1, RECIPIENT);
    assert_eq!(
        kind(evaluate(&policy(), &automation(), &q, &[], NOW)),
        RefusalKind::AssetNotAllowed
    );
}

#[test]
fn a_pinned_token_address_refuses_a_same_symbol_impostor() {
    let mut policy = policy();
    policy.assets[0].token_address = Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into());
    let mut q = quote("q", "USDC", 1, RECIPIENT);
    q.token_address = Some("Fake1111111111111111111111111111111111111111".into());
    assert_eq!(
        kind(evaluate(&policy, &automation(), &q, &[], NOW)),
        RefusalKind::AssetNotAllowed
    );
    q.token_address = Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into());
    assert_eq!(evaluate(&policy, &automation(), &q, &[], NOW), Ok(true));
}

#[test]
fn automation_is_refused_when_the_owner_has_not_enabled_it() {
    let mut policy = policy();
    policy.automation_enabled = false;
    let q = quote("q", "USDC", 1, RECIPIENT);
    assert_eq!(
        kind(evaluate(&policy, &automation(), &q, &[], NOW)),
        RefusalKind::AutomationDisabled
    );
    assert_eq!(
        kind(evaluate(
            &WalletPolicyConfig::default(),
            &automation(),
            &q,
            &[],
            NOW
        )),
        RefusalKind::AutomationDisabled
    );
}

#[test]
fn interactive_executes_stay_uncapped_by_default_and_capped_when_configured() {
    let q = quote("q", "BONK", u64::MAX, RECIPIENT);
    assert_eq!(
        evaluate(&policy(), &ExecutionOrigin::rpc(), &q, &[], NOW),
        Ok(false)
    );
    assert_eq!(
        evaluate(
            &WalletPolicyConfig::default(),
            &ExecutionOrigin::rpc(),
            &q,
            &[],
            NOW
        ),
        Ok(false)
    );
    let mut strict = policy();
    strict.apply_to_interactive = true;
    assert_eq!(
        kind(evaluate(&strict, &ExecutionOrigin::rpc(), &q, &[], NOW)),
        RefusalKind::AssetNotAllowed
    );
}

#[test]
fn turn_origins_map_to_interactive_automation_or_refused() {
    use crate::core::runtime::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};
    let trusted = |source| AgentTurnOrigin::TrustedAutomation {
        job_id: "j".into(),
        source,
    };
    assert!(matches!(
        ExecutionOrigin::for_turn(Some(&trusted(TrustedAutomationSource::Background))),
        ExecutionOrigin::Automation { .. }
    ));
    assert!(matches!(
        ExecutionOrigin::for_turn(Some(&trusted(TrustedAutomationSource::Workflow {
            require_approval: false
        }))),
        ExecutionOrigin::Automation { .. }
    ));
    assert!(matches!(
        ExecutionOrigin::for_turn(Some(&trusted(TrustedAutomationSource::CronActions))),
        ExecutionOrigin::Automation { .. }
    ));
    assert!(matches!(
        ExecutionOrigin::for_turn(Some(&trusted(TrustedAutomationSource::Workflow {
            require_approval: true
        }))),
        ExecutionOrigin::Agent { .. }
    ));
    assert!(matches!(
        ExecutionOrigin::for_turn(Some(&AgentTurnOrigin::WebChat {
            thread_id: "t".into(),
            client_id: "c".into(),
            request_id: None
        })),
        ExecutionOrigin::Agent { .. }
    ));
    let channel = AgentTurnOrigin::ExternalChannel {
        channel: "telegram".into(),
        sender: None,
        sender_name: None,
        reply_target: "r".into(),
        message_id: "m".into(),
        history_key: None,
        channel_session: None,
    };
    for origin in [Some(&channel), Some(&AgentTurnOrigin::Unknown), None] {
        let classified = ExecutionOrigin::for_turn(origin);
        assert!(matches!(classified, ExecutionOrigin::Refused { .. }));
        let q = quote("q", "USDC", 1, RECIPIENT);
        assert_eq!(
            kind(evaluate(&policy(), &classified, &q, &[], NOW)),
            RefusalKind::Origin
        );
    }
}

fn executed(q: &PreparedTransaction) -> ExecutionResult {
    ExecutionResult {
        quote_id: q.quote_id.clone(),
        status: PreparedStatus::Broadcasted,
        chain: q.chain,
        evm_network: None,
        transaction_hash: "5igNature".into(),
        explorer_url: None,
        transaction: q.clone(),
    }
}

#[tokio::test]
async fn guarded_execute_records_pending_before_broadcast_and_broadcast_after() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = SpendLedger::open(dir.path()).unwrap();
    let q = quote("q-ok", "USDC", 10_000_000, RECIPIENT);
    let observed = ledger.clone();
    let result = guarded_execute(&ledger, &policy(), &automation(), &q, NOW, || async {
        // The record exists, pending, while the module is signing.
        let records = observed.records().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, LedgerStatus::Pending);
        Ok(executed(&q))
    })
    .await
    .unwrap();
    assert_eq!(result.transaction_hash, "5igNature");
    let records = ledger.records().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, LedgerStatus::Broadcast);
    assert_eq!(records[0].transaction_hash.as_deref(), Some("5igNature"));
    assert!(records[0].policy_applied);
}

#[tokio::test]
async fn an_in_doubt_execute_is_never_retried_for_the_same_quote() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = SpendLedger::open(dir.path()).unwrap();
    let q = quote("q-doubt", "USDC", 10_000_000, RECIPIENT);
    let calls = AtomicUsize::new(0);
    let first = guarded_execute(&ledger, &policy(), &automation(), &q, NOW, || async {
        calls.fetch_add(1, Ordering::SeqCst);
        Err(WalletCallError::Failed("rpc timed out after send".into()))
    })
    .await;
    assert!(first.is_err());
    assert_eq!(ledger.records().unwrap()[0].status, LedgerStatus::InDoubt);

    // Even an interactive retry of the same quote is refused without calling
    // the module.
    let second = guarded_execute(
        &ledger,
        &policy(),
        &ExecutionOrigin::rpc(),
        &q,
        NOW,
        || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(executed(&q))
        },
    )
    .await
    .unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(second.to_string().starts_with(POLICY_REFUSED_PREFIX));
    assert!(second.to_string().contains("never retried"));
}

#[tokio::test]
async fn an_over_cap_automation_execute_is_refused_recorded_and_never_reaches_the_module() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = SpendLedger::open(dir.path()).unwrap();
    let q = quote("q-big", "USDC", 60_000_000, RECIPIENT);
    let error = guarded_execute(&ledger, &policy(), &automation(), &q, NOW, || async {
        panic!("the module must not be called for an over-cap execute")
    })
    .await
    .unwrap_err();
    assert!(matches!(&error, WalletCallError::InvalidInput(message)
        if message.starts_with(POLICY_REFUSED_PREFIX) && message.contains("per-transaction")));
    let records = ledger.records().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, LedgerStatus::Refused);
}

#[tokio::test]
async fn spend_recorded_before_a_restart_still_counts_against_the_daily_cap() {
    let dir = tempfile::tempdir().unwrap();
    {
        let ledger = SpendLedger::open(dir.path()).unwrap();
        let q = quote("q-1", "USDC", 50_000_000, "A");
        guarded_execute(&ledger, &policy(), &automation(), &q, NOW, || async {
            Ok(executed(&q))
        })
        .await
        .unwrap();
        let q = quote("q-2", "USDC", 50_000_000, "B");
        guarded_execute(&ledger, &policy(), &automation(), &q, NOW, || async {
            Ok(executed(&q))
        })
        .await
        .unwrap();
    }
    let reopened = SpendLedger::open(dir.path()).unwrap();
    let q = quote("q-3", "USDC", 1, "C");
    let error = guarded_execute(&reopened, &policy(), &automation(), &q, NOW + 1, || async {
        Ok(executed(&q))
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("24h"));
}

#[tokio::test]
async fn an_interactive_execute_keeps_working_uncapped_and_is_still_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = SpendLedger::open(dir.path()).unwrap();
    let q = quote("q-ui", "SOL", 9_000_000_000, RECIPIENT);
    guarded_execute(
        &ledger,
        &policy(),
        &ExecutionOrigin::rpc(),
        &q,
        NOW,
        || async { Ok(executed(&q)) },
    )
    .await
    .unwrap();
    let records = ledger.records().unwrap();
    assert_eq!(records[0].origin, "interactive");
    assert!(!records[0].policy_applied);
    assert_eq!(records[0].status, LedgerStatus::Broadcast);
}

#[test]
fn an_agent_execute_in_a_chat_is_always_capped_even_without_automation_enabled() {
    let chat = ExecutionOrigin::Agent {
        class: "WebChat".into(),
    };
    let mut policy = policy();
    policy.automation_enabled = false;
    let q = quote("q", "USDC", 1, RECIPIENT);
    assert_eq!(evaluate(&policy, &chat, &q, &[], NOW), Ok(true));
    let big = quote("q", "USDC", 50_000_001, RECIPIENT);
    assert_eq!(
        kind(evaluate(&policy, &chat, &big, &[], NOW)),
        RefusalKind::PerTransaction
    );
    // With no policy configured, the agent cannot execute at all.
    assert_eq!(
        kind(evaluate(
            &WalletPolicyConfig::default(),
            &chat,
            &q,
            &[],
            NOW
        )),
        RefusalKind::AssetNotAllowed
    );
}

#[test]
fn a_scheduled_actions_cron_turn_is_automation_capped_by_the_policy() {
    use crate::core::runtime::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};
    let origin = ExecutionOrigin::for_turn(Some(&AgentTurnOrigin::TrustedAutomation {
        job_id: "j".into(),
        source: TrustedAutomationSource::CronActions,
    }));
    assert!(matches!(origin, ExecutionOrigin::Automation { .. }));
    let under = quote("q", "USDC", 1_000_000, RECIPIENT);
    assert_eq!(evaluate(&policy(), &origin, &under, &[], NOW), Ok(true));
    let over = quote("q2", "USDC", 60_000_000, RECIPIENT);
    assert_eq!(
        kind(evaluate(&policy(), &origin, &over, &[], NOW)),
        RefusalKind::PerTransaction
    );
    let mut disabled = policy();
    disabled.automation_enabled = false;
    assert_eq!(
        kind(evaluate(&disabled, &origin, &under, &[], NOW)),
        RefusalKind::AutomationDisabled
    );
}

#[test]
fn a_plain_cron_turn_is_refused_by_the_spend_policy() {
    use crate::core::runtime::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};
    let origin = ExecutionOrigin::for_turn(Some(&AgentTurnOrigin::TrustedAutomation {
        job_id: "j".into(),
        source: TrustedAutomationSource::Cron,
    }));
    assert!(matches!(origin, ExecutionOrigin::Refused { .. }));
    let q = quote("q", "USDC", 1, RECIPIENT);
    assert_eq!(
        kind(evaluate(&policy(), &origin, &q, &[], NOW)),
        RefusalKind::Origin
    );
}
